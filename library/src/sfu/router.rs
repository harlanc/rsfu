use super::simulcast::SimulcastConfig;

use super::down_track::DownTrack;
use super::errors::Result;
use super::receiver::Receiver;
use super::receiver::WebRTCReceiver;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use super::subscriber::Subscriber;
use crate::buffer::factory::AtomicFactory;
use crate::stats::stream::Stream;
use crate::twcc::twcc::Responder;
use async_trait::async_trait;
use rtcp::packet::Packet as RtcpPacket;
use rtcp::raw_packet::RawPacket;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;

use tokio::sync::{broadcast, mpsc, oneshot};

pub type RtcpDataSender = mpsc::UnboundedSender<Vec<Box<dyn RtcpPacket + Send + Sync>>>;
pub type RtcpDataReceiver = mpsc::UnboundedReceiver<Vec<Box<dyn RtcpPacket + Send + Sync>>>;

pub type RtcpWriterFn = Box<
    dyn (FnMut(
            Vec<Box<dyn RtcpPacket + Send + Sync>>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnAddReciverTrackFn = Box<
    dyn (FnMut(
            Arc<Mutex<dyn Receiver + Send + Sync>>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;
pub type OnDelReciverTrackFn = Box<
    dyn (FnMut(
            Arc<Mutex<dyn Receiver + Send + Sync>>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;

#[async_trait]
pub trait Router {
    fn id(&self) -> String;
    async fn add_receiver(
        &mut self,
        receiver: Arc<RTCRtpReceiver>,
        track: Arc<TrackRemote>,
        track_id: String,
        stream_id: String,
    ) -> (Arc<Mutex<dyn Receiver + Send + Sync>>, bool);
    fn add_down_tracks(
        &mut self,
        s: Arc<Subscriber>,
        r: Arc<Mutex<dyn Receiver + Send + Sync>>,
    ) -> Result<()>;
    fn add_down_track(
        &mut self,
        s: Arc<Subscriber>,
        r: Arc<Mutex<dyn Receiver + Send + Sync>>,
    ) -> Result<Arc<Option<DownTrack>>>;
    fn set_rtcp_writer(&self, writer: RtcpWriterFn);
    fn get_receiver(&self) -> HashMap<String, Arc<Mutex<dyn Receiver + Send + Sync>>>;
    fn set_peer_connection(&mut self, pc: Arc<RTCPeerConnection>);
    async fn stop(&self);
    async fn on_add_receiver_track(&self, f: OnAddReciverTrackFn);
    async fn on_del_receiver_track(&self, f: OnDelReciverTrackFn);
}

#[derive(Default, Clone)]
pub struct RouterConfig {
    pub(super) with_stats: bool,
    max_bandwidth: u64,
    pub max_packet_track: i32,
    pub audio_level_interval: i32,
    audio_level_threshold: u8,
    audio_level_filter: i32,
    simulcast: SimulcastConfig,
}

pub struct RouterLocal {
    id: String,
    twcc: Arc<Mutex<Option<Responder>>>,
    stats: Arc<Mutex<HashMap<u32, Stream>>>,
    rtcp_sender_channel: Arc<RtcpDataSender>,
    rtcp_receiver_channel: Arc<Mutex<RtcpDataReceiver>>,
    stop_sender_channel: mpsc::UnboundedSender<()>,
    stop_receiver_channel: mpsc::UnboundedReceiver<()>,
    config: RouterConfig,
    session: Arc<Mutex<dyn Session + Send + Sync>>,
    receivers: HashMap<String, Arc<Mutex<dyn Receiver + Send + Sync>>>,
    buffer_factory: AtomicFactory,
    rtcp_writer_handler: Arc<Mutex<Option<RtcpWriterFn>>>,
    on_add_receiver_track_handler: Arc<Mutex<Option<OnAddReciverTrackFn>>>,
    on_del_receiver_track_handler: Arc<Mutex<Option<OnDelReciverTrackFn>>>,
}
impl RouterLocal {
    pub fn new(
        id: String,
        session: Arc<Mutex<dyn Session + Send + Sync>>,
        config: RouterConfig,
    ) -> Self {
        let (s, r) = mpsc::unbounded_channel();
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            id,
            twcc: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(HashMap::new())),
            rtcp_sender_channel: Arc::new(s),
            rtcp_receiver_channel: Arc::new(Mutex::new(r)),
            stop_sender_channel: sender,
            stop_receiver_channel: receiver,
            config,
            session,
            receivers: HashMap::new(),
            buffer_factory: AtomicFactory::new(100, 100),
            rtcp_writer_handler: Arc::new(Mutex::new(None)),
            on_add_receiver_track_handler: Arc::new(Mutex::new(None)),
            on_del_receiver_track_handler: Arc::new(Mutex::new(None)),
        }
    }

    async fn delete_receiver(&mut self, track: String, ssrc: u32) {
        if let Some(f) = &mut *self.on_del_receiver_track_handler.lock().await {
            if let Some(track) = self.receivers.get(&track) {
                f(track.clone());
            }
        }
        self.receivers.remove(&track);
        self.stats.lock().await.remove(&ssrc);
    }
    async fn send_rtcp(&mut self) {
        loop {
            let mut rtcp_receiver = self.rtcp_receiver_channel.lock().await;
            tokio::select! {
              data = rtcp_receiver.recv() => {
                if let Some(val) = data{
                    if let Some(f) = &mut *self.rtcp_writer_handler.lock().await {
                        f(val);
                    }
                }
              }
              data = self.stop_receiver_channel.recv() => {
                return ;
              }
            };
        }
    }
}

#[async_trait]
impl Router for RouterLocal {
    fn get_receiver(&self) -> HashMap<String, Arc<Mutex<dyn Receiver + Send + Sync>>> {
        self.receivers.clone()
    }

    async fn on_add_receiver_track(&self, f: OnAddReciverTrackFn) {
        let mut handler = self.on_add_receiver_track_handler.lock().await;
        *handler = Some(f);
    }
    async fn on_del_receiver_track(&self, f: OnDelReciverTrackFn) {
        let mut handler = self.on_del_receiver_track_handler.lock().await;
        *handler = Some(f);
    }

    fn id(&self) -> String {
        self.id.clone()
    }

    async fn stop(&self) {
        let rv = self.stop_sender_channel.send(());
        if self.config.with_stats {}
    }

    async fn add_receiver(
        &mut self,
        receiver: Arc<RTCRtpReceiver>,
        track: Arc<TrackRemote>,
        track_id: String,
        stream_id: String,
    ) -> (Arc<Mutex<dyn Receiver + Send + Sync>>, bool) {
        let mut publish = false;

        let buffer = self.buffer_factory.get_or_new_buffer(track.ssrc()).await;

        let sender_for_buffer = Arc::clone(&self.rtcp_sender_channel);
        buffer
            .lock()
            .await
            .on_feedback_callback(Box::new(
                move |packets: Vec<Box<dyn RtcpPacket + Send + Sync>>| {
                    let sender_for_buffer_in = Arc::clone(&sender_for_buffer);
                    Box::pin(async move {
                        sender_for_buffer_in.send(packets);
                    })
                },
            ))
            .await;

        match track.kind() {
            RTPCodecType::Audio => {
                let session_out = Arc::clone(&self.session);
                let stream_id_out = stream_id.clone();
                buffer
                    .lock()
                    .await
                    .on_audio_level(Box::new(move |level: u8| {
                        let session_in = Arc::clone(&session_out);
                        let stream_id_in = stream_id_out.clone();
                        Box::pin(async move {
                            if let Some(observer) = session_in.lock().await.audio_obserber() {
                                observer.observe(stream_id_in, level).await;
                            }
                        })
                    }))
                    .await;
                if let Some(observer) = self.session.lock().await.audio_obserber() {
                    observer.add_stream(stream_id).await;
                }
            }
            RTPCodecType::Video => {
                if self.twcc.lock().await.is_none() {
                    let mut twcc = Responder::new(track.ssrc());
                    let sender = Arc::clone(&self.rtcp_sender_channel);
                    twcc.on_feedback(Box::new(
                        move |rtcp_packet: Box<dyn RtcpPacket + Send + Sync>| {
                            let sender_in = Arc::clone(&sender);
                            Box::pin(async move {
                                let mut data = Vec::new();
                                data.push(rtcp_packet);
                                sender_in.send(data);
                            })
                        },
                    ))
                    .await;
                    self.twcc = Arc::new(Mutex::new(Some(twcc)));
                }

                let twcc_out = Arc::clone(&self.twcc);
                buffer
                    .lock()
                    .await
                    .on_transport_wide_cc(Box::new(move |sn: u16, time_ns: i64, marker: bool| {
                        let twcc_in = Arc::clone(&twcc_out);
                        Box::pin(async move {
                            if let Some(twcc) = &mut *twcc_in.lock().await {
                                twcc.push(sn, time_ns, marker).await;
                            }
                        })
                    }))
                    .await;
            }
            RTPCodecType::Unspecified => {}
        }

        if self.config.with_stats {
            let stream = Stream::new(Arc::clone(&buffer));
            self.stats.lock().await.insert(track.ssrc(), stream);
        }

        let rtcp_reader = self
            .buffer_factory
            .get_or_new_rtcp_buffer(track.ssrc())
            .await;

        let stats_out = Arc::clone(&self.stats);
        let buffer_out = Arc::clone(&buffer);
        let with_status = self.config.with_stats;

        rtcp_reader
            .lock()
            .await
            .on_packet(Box::new(move |packet: Vec<u8>| {
                let stats_in = Arc::clone(&stats_out);
                let buffer_in = Arc::clone(&buffer_out);
                Box::pin(async move {
                    let mut buf = &packet[..];
                    let pkts_result = rtcp::packet::unmarshal(&mut buf)?;
                    for pkt in pkts_result {
                        if let Some(source_description) =
                            pkt.as_any()
                                .downcast_ref::<rtcp::source_description::SourceDescription>()
                        {
                            if with_status {
                                for chunk in &source_description.chunks {
                                    if let Some(stream) =
                                        stats_in.lock().await.get_mut(&chunk.source)
                                    {
                                        for item in &chunk.items {
                                            if item.sdes_type
                                                == rtcp::source_description::SdesType::SdesCname
                                            {
                                                stream
                                                    .set_cname(
                                                        String::from_utf8(item.text.to_vec())
                                                            .unwrap(),
                                                    )
                                                    .await;
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Some(sender_report) =
                            pkt.as_any()
                                .downcast_ref::<rtcp::sender_report::SenderReport>()
                        {
                            buffer_in.lock().await.set_sender_report_data(
                                sender_report.rtp_time,
                                sender_report.ntp_time,
                            );
                            if with_status {
                                if let Some(stream) =
                                    stats_in.lock().await.get_mut(&sender_report.ssrc)
                                {
                                    //update stats
                                }
                            }
                        }
                    }
                    Ok(())
                })
            }))
            .await;

        let mut result_receiver;

        if let Some(recv) = self.receivers.get(&track_id) {
            result_receiver = recv.clone();
        } else {
            let mut rv = WebRTCReceiver::new(receiver, track.clone(), self.id.clone()).await;
            rv.set_rtcp_channel(self.rtcp_sender_channel.clone());
            let recv_kind = rv.kind();
            let session_out = self.session.clone();
            let stream_id = track.stream_id().await;
            rv.on_close_handler(Box::new(move || {
                //let stats_in = Arc::clone(&stats_out);
                let session_in = session_out.clone();
                let stream_id_in = stream_id.clone();
                Box::pin(async move {
                    if with_status {
                        // match track.kind() {
                        //     RTPCodecType::Video => {
                        //         //todo
                        //     }
                        //     _ => {
                        //         //todo
                        //     }
                        // }
                    }
                    if recv_kind == RTPCodecType::Audio {
                        if let Some(audio_observer) = session_in.lock().await.audio_obserber() {
                            audio_observer.remove_stream(stream_id_in).await;
                        }
                    }
                })
            }))
            .await;
            result_receiver = Arc::new(Mutex::new(rv));

            self.receivers.insert(track_id, result_receiver.clone());
        }

        (result_receiver, publish)
    }

    fn add_down_tracks(
        &mut self,
        s: Arc<Subscriber>,
        r: Arc<Mutex<dyn Receiver + Send + Sync>>,
    ) -> Result<()> {
        Ok(())
    }

    fn add_down_track(
        &mut self,
        s: Arc<Subscriber>,
        r: Arc<Mutex<dyn Receiver + Send + Sync>>,
    ) -> Result<(Arc<Option<DownTrack>>)> {
        Ok(Arc::new(None))
    }

    fn set_rtcp_writer(&self, writer: RtcpWriterFn) {}

    fn set_peer_connection(&mut self, pc: Arc<RTCPeerConnection>) {
        // Ok(());
    }
}
