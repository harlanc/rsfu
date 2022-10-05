use super::simulcast::SimulcastConfig;

use super::down_track::DownTrack;
use super::errors::Result;
use super::receiver::Receiver;
use super::receiver::WebRTCReceiver;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use super::subscriber::Subscriber;
use crate::buffer::factory::AtomicFactory;
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

pub type RtcpWriterFn = Box<
    dyn (FnMut(
            Vec<Box<dyn RtcpPacket + Send + Sync>>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnAddReciverTrackFn = Box<
    dyn (FnMut(Arc<RTCRtpReceiver>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;
pub type OnDelReciverTrackFn = Box<
    dyn (FnMut(Arc<RTCRtpReceiver>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
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
    async fn stop(&mut self);
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
    rtcp_channel: Arc<RtcpDataSender>,
    stop_channel: mpsc::Sender<()>,
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
        let (sender, _) = mpsc::channel(1);
        Self {
            id,
            twcc: Arc::new(Mutex::new(None)),
            rtcp_channel: Arc::new(s),
            stop_channel: sender,
            config,
            session,
            receivers: HashMap::new(),
            buffer_factory: AtomicFactory::new(100, 100),
            rtcp_writer_handler: Arc::new(Mutex::new(None)),
            on_add_receiver_track_handler: Arc::new(Mutex::new(None)),
            on_del_receiver_track_handler: Arc::new(Mutex::new(None)),
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

    async fn stop(&mut self) {
        let rv = self.stop_channel.send(()).await;
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

        let sender_for_buffer = Arc::clone(&self.rtcp_channel);
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
                    let sender = Arc::clone(&self.rtcp_channel);
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
        let mut result_receiver;

        if let Some(recv) = self.receivers.get(&track_id) {
            result_receiver = recv.clone();
        } else {
            let rv = WebRTCReceiver::new(receiver, track, self.id.clone()).await;
            result_receiver = Arc::new(Mutex::new(rv));
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
