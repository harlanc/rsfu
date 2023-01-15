use super::simulcast::SimulcastConfig;

use super::down_track::DownTrack;
use super::down_track_local::DownTrackLocal;
use super::errors::Result;
use super::receiver::Receiver;
use super::receiver::WebRTCReceiver;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use super::subscriber::Subscriber;
use crate::buffer::buffer::Options as BufferOptions;
use crate::buffer::buffer_io::BufferIO;
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
use webrtc::error::Error as RTCError;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::rtp_transceiver::{RTCRtpTransceiverInit, SSRC};
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_remote::TrackRemote;

use serde::Deserialize;
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
            Arc<dyn Receiver + Send + Sync>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;
pub type OnDelReciverTrackFn = Box<
    dyn (FnMut(
            Arc<dyn Receiver + Send + Sync>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;

#[async_trait]
pub trait Router {
    fn id(&self) -> String;
    async fn add_receiver(
        &self,
        receiver: Arc<RTCRtpReceiver>,
        track: Arc<TrackRemote>,
        track_id: String,
        stream_id: String,
    ) -> (Arc<dyn Receiver + Send + Sync>, bool);
    async fn add_down_tracks(
        &self,
        s: Arc<Subscriber>,
        r: Option<Arc<dyn Receiver + Send + Sync>>,
    ) -> Result<()>;
    async fn add_down_track(
        &self,
        s: Arc<Subscriber>,
        r: Arc<dyn Receiver + Send + Sync>,
    ) -> Result<Option<Arc<DownTrack>>>;
    async fn set_rtcp_writer(&self, writer: RtcpWriterFn);
    fn get_receiver(&self) -> Arc<Mutex<HashMap<String, Arc<dyn Receiver + Send + Sync>>>>;

    async fn stop(&self);
    async fn on_add_receiver_track(&self, f: OnAddReciverTrackFn);
    async fn on_del_receiver_track(&self, f: OnDelReciverTrackFn);
    async fn send_rtcp(&self);
}

#[derive(Default, Clone, Deserialize)]
pub struct RouterConfig {
    #[serde(rename = "withstats")]
    pub(super) with_stats: bool,
    #[serde(rename = "maxbandwidth")]
    max_bandwidth: u64,
    #[serde(rename = "maxpackettrack")]
    pub max_packet_track: i32,
    #[serde(rename = "audiolevelinterval")]
    pub audio_level_interval: i32,
    #[serde(rename = "audiolevelthreshold")]
    audio_level_threshold: u8,
    #[serde(rename = "audiolevelfilter")]
    audio_level_filter: i32,
    simulcast: SimulcastConfig,
}

pub struct RouterLocal {
    id: String,
    twcc: Arc<Mutex<Option<Responder>>>,
    stats: Arc<Mutex<HashMap<u32, Stream>>>,
    rtcp_sender_channel: Arc<RtcpDataSender>,
    rtcp_receiver_channel: Arc<Mutex<RtcpDataReceiver>>,
    stop_sender_channel: Arc<Mutex<mpsc::UnboundedSender<()>>>,
    stop_receiver_channel: Arc<Mutex<mpsc::UnboundedReceiver<()>>>,
    config: RouterConfig,
    session: Arc<dyn Session + Send + Sync>,
    receivers: Arc<Mutex<HashMap<String, Arc<dyn Receiver + Send + Sync>>>>,
    buffer_factory: AtomicFactory,
    rtcp_writer_handler: Arc<Mutex<Option<RtcpWriterFn>>>,
    on_add_receiver_track_handler: Arc<Mutex<Option<OnAddReciverTrackFn>>>,
    on_del_receiver_track_handler: Arc<Mutex<Option<OnDelReciverTrackFn>>>,
}
impl RouterLocal {
    pub fn new(id: String, session: Arc<dyn Session + Send + Sync>, config: RouterConfig) -> Self {
        let (s, r) = mpsc::unbounded_channel();
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            id,
            twcc: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(HashMap::new())),
            rtcp_sender_channel: Arc::new(s),
            rtcp_receiver_channel: Arc::new(Mutex::new(r)),
            stop_sender_channel: Arc::new(Mutex::new(sender)),
            stop_receiver_channel: Arc::new(Mutex::new(receiver)),
            config,
            session,
            receivers: Arc::new(Mutex::new(HashMap::new())),
            buffer_factory: AtomicFactory::new(100, 100),
            rtcp_writer_handler: Arc::new(Mutex::new(None)),
            on_add_receiver_track_handler: Arc::new(Mutex::new(None)),
            on_del_receiver_track_handler: Arc::new(Mutex::new(None)),
        }
    }

    async fn delete_receiver(&self, track: String, ssrc: u32) {
        if let Some(f) = &mut *self.on_del_receiver_track_handler.lock().await {
            if let Some(track) = self.receivers.lock().await.get(&track) {
                f(track.clone());
            }
        }
        self.receivers.lock().await.remove(&track);
        self.stats.lock().await.remove(&ssrc);
    }
}

#[async_trait]
impl Router for RouterLocal {
    fn get_receiver(&self) -> Arc<Mutex<HashMap<String, Arc<dyn Receiver + Send + Sync>>>> {
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
        let rv = self.stop_sender_channel.lock().await.send(());
        if self.config.with_stats {}
    }

    async fn add_receiver(
        &self,
        receiver: Arc<RTCRtpReceiver>,
        track: Arc<TrackRemote>,
        track_id: String,
        stream_id: String,
    ) -> (Arc<dyn Receiver + Send + Sync>, bool) {
        let mut publish = false;

        let buffer = self.buffer_factory.get_or_new_buffer(track.ssrc()).await;

        let sender_for_buffer = Arc::clone(&self.rtcp_sender_channel);
        buffer
            .on_feedback_callback(Box::new(
                move |packets: Vec<Box<dyn RtcpPacket + Send + Sync>>| {
                    let sender_for_buffer_in = Arc::clone(&sender_for_buffer);
                    Box::pin(async move {
                        sender_for_buffer_in.send(packets);
                    })
                },
            ))
            .await;
        //println!("add_receiver 0....");
        match track.kind() {
            RTPCodecType::Audio => {
                let session_out = Arc::clone(&self.session);
                let stream_id_out = stream_id.clone();
                buffer
                    .on_audio_level(Box::new(move |level: u8| {
                        let session_in = Arc::clone(&session_out);
                        let stream_id_in = stream_id_out.clone();
                        Box::pin(async move {
                            if let Some(observer) = session_in.audio_obserber() {
                                observer.lock().await.observe(stream_id_in, level).await;
                            }
                        })
                    }))
                    .await;
                if let Some(observer) = self.session.audio_obserber() {
                    observer.lock().await.add_stream(stream_id).await;
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
                    let mut t = &*self.twcc.lock().await;
                    t = &Some(twcc);
                    //self.twcc = Arc::new(Mutex::new(Some(twcc)));
                }

                let twcc_out = Arc::clone(&self.twcc);
                buffer
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
        //println!("add_receiver 1....");
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
        //println!("add_receiver 2....");
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
                            buffer_in.set_sender_report_data(
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

        let result_receiver;
        //println!("add_receiver 3....");
        let mut receivers = self.receivers.lock().await;
        if let Some(recv) = receivers.get(&track_id) {
            //println!("add_receiver 3.1....");
            result_receiver = recv.clone();
        } else {
            // println!("add_receiver 3.2....");
            let mut rv =
                WebRTCReceiver::new(receiver.clone(), track.clone(), self.id.clone()).await;
            rv.set_rtcp_channel(self.rtcp_sender_channel.clone());
            let recv_kind = rv.kind();
            let session_out = self.session.clone();
            let stream_id = track.stream_id().await;
            //println!("add_receiver 3.3....");
            let receivers_out = self.receivers.clone();
            let stats_out = self.stats.clone();
            let del_handler_out = self.on_add_receiver_track_handler.clone();
            let track_id_out = track_id.clone();
            let track_ssrc = track.ssrc();
            rv.on_close_handler(Box::new(move || {
                //let stats_in = Arc::clone(&stats_out);
                let session_in = session_out.clone();
                let stream_id_in = stream_id.clone();
                let track_id_in = track_id_out.clone();

                let receivers_in = receivers_out.clone();
                let stats_in = stats_out.clone();
                let del_handler_in = del_handler_out.clone();

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
                        if let Some(audio_observer) = session_in.audio_obserber() {
                            audio_observer
                                .lock()
                                .await
                                .remove_stream(stream_id_in)
                                .await;
                        }
                    }
                    delete_receiver(
                        &track_id_in,
                        &track_ssrc,
                        del_handler_in,
                        receivers_in,
                        stats_in,
                    )
                    .await;
                })
            }))
            .await;
            result_receiver = Arc::new(rv);
            //println!("add_receiver 3.4....");
            receivers.insert(track_id, result_receiver.clone());

            publish = true;
            //println!("add_receiver 3.5....");
            if let Some(f) = &mut *self.on_add_receiver_track_handler.lock().await {
                println!("add_receiver 3.6....");
                f(result_receiver.clone());
            }
        }
        //println!("add_receiver 4....");
        let layer = result_receiver
            .add_up_track(
                track.clone(),
                buffer.clone(),
                self.config.simulcast.best_quality_first,
            )
            .await;

        if let Some(layer_val) = layer {
            let receiver_clone = result_receiver.clone();
            tokio::spawn(async move { receiver_clone.write_rtp(layer_val).await });
        }

        buffer
            .bind(
                receiver.get_parameters().await,
                BufferOptions {
                    max_bitrate: self.config.max_bandwidth,
                },
            )
            .await;

        let track_clone = track.clone();
        let buffer_clone = buffer.clone();
        println!("add_receiver 5....");

        tokio::spawn(async move {
            let mut b = vec![0u8; 1500];
            println!("read begin....");
            while let Ok((n, _)) = track_clone.read(&mut b).await {
                //println!("read size:{}", n);
                buffer_clone.write(&b[..n]).await;

                // Unmarshal the packet and update the PayloadType
                //   let mut buf = &b[..n];
                // let mut rtp_packet = webrtc::rtp::packet::Packet::unmarshal(&mut buf)?;
                // rtp_packet.header.payload_type = c.payload_type;

                // // Marshal into original buffer with updated PayloadType

                // let n = rtp_packet.marshal_to(&mut b)?;

                // // Write
                // if let Err(err) = c.conn.send(&b[..n]).await {
                //     // For this particular example, third party applications usually timeout after a short
                //     // amount of time during which the user doesn't have enough time to provide the answer
                //     // to the browser.
                //     // That's why, for this particular example, the user first needs to provide the answer
                //     // to the browser then open the third party application. Therefore we must not kill
                //     // the forward on "connection refused" errors
                //     //if opError, ok := err.(*net.OpError); ok && opError.Err.Error() == "write: connection refused" {
                //     //    continue
                //     //}
                //     //panic(err)
                //     if err.to_string().contains("Connection refused") {
                //         continue;
                //     } else {
                //         println!("conn send err: {}", err);
                //         break;
                //     }
                // }
            }

            Result::<()>::Ok(())
        });
        (result_receiver, publish)
    }

    async fn add_down_tracks(
        &self,
        s: Arc<Subscriber>,
        r: Option<Arc<dyn Receiver + Send + Sync>>,
    ) -> Result<()> {
        if s.no_auto_subscribe {
            return Ok(());
        }

        if let Some(receiver) = r {
            self.add_down_track(s.clone(), receiver).await?;
            log::info!("AddDownTracks  Negotiate");
            s.negotiate().await;
            return Ok(());
        }

        let mut recs = Vec::new();
        {
            let mut receivers = self.receivers.lock().await;
            for (_, receiver) in &mut *receivers {
                recs.push(receiver.clone())
            }
        }

        if recs.len() > 0 {
            for val in recs {
                self.add_down_track(s.clone(), val.clone()).await?;
            }
            log::info!("AddDownTracks 2 Negotiate");
            s.negotiate().await;
        }

        Ok(())
    }

    async fn add_down_track(
        &self,
        s: Arc<Subscriber>,
        r: Arc<dyn Receiver + Send + Sync>,
    ) -> Result<(Option<Arc<DownTrack>>)> {
        let mut recv = r.clone();

        let downtracks = s.get_downtracks(recv.stream_id()).await;
        log::info!("add_down_track 0..");
        if let Some(downtracks_data) = downtracks {
            for dt in downtracks_data {
                if dt.id() == recv.track_id() {
                    return Ok(Some(dt));
                }
            }
        }
        log::info!("add_down_track 1..");
        let codec = recv.codec();
        s.me.lock()
            .await
            .register_codec(codec.clone(), recv.kind())?;
        log::info!("add_down_track 2..");
        let codec_capability = RTCRtpCodecCapability {
            mime_type: codec.capability.mime_type,
            clock_rate: codec.capability.clock_rate,
            channels: codec.capability.channels,
            sdp_fmtp_line: codec.capability.sdp_fmtp_line,
            rtcp_feedback: vec![
                RTCPFeedback {
                    typ: String::from("goog-remb"),
                    parameter: String::from(""),
                },
                RTCPFeedback {
                    typ: String::from("nack"),
                    parameter: String::from(""),
                },
                RTCPFeedback {
                    typ: String::from("nack"),
                    parameter: String::from("pli"),
                },
            ],
        };

        let down_track_local =
            DownTrackLocal::new(codec_capability, r.clone(), self.config.max_packet_track).await;

        let down_track_arc = Arc::new(down_track_local);

        let transceiver =
            s.pc.add_transceiver_from_track(
                down_track_arc.clone(),
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendonly,
                    send_encodings: Vec::new(),
                }),
            )
            .await?;
        log::info!("add_down_track 3..");
        let mut down_track = DownTrack::new_track_local(s.id.clone(), down_track_arc);
        down_track.set_transceiver(transceiver.clone());

        let down_track_arc = Arc::new(down_track);

        let s_out = s.clone();
        let r_out = r.clone();
        let transceiver_out = transceiver.clone();
        let down_track_arc_out = down_track_arc.clone();

        down_track_arc
            .on_close_handler(Box::new(move || {
                let s_in = s_out.clone();
                let r_in = r_out.clone();
                let transceiver_in = transceiver_out.clone();
                let down_track_arc_in = down_track_arc_out.clone();
                Box::pin(async move {
                    if s_in.pc.connection_state() != RTCPeerConnectionState::Closed {
                        let rv = s_in
                            .pc
                            .remove_track(&transceiver_in.sender().await)
                            .await;
                        match rv {
                            Ok(_) => {
                                s_in.remove_down_track(r_in.stream_id(), down_track_arc_in)
                                    .await;
                                log::info!("RemoveDownTrack Negotiate");
                                s_in.negotiate().await;
                            }
                            Err(err) => match err {
                                RTCError::ErrConnectionClosed => {
                                    return;
                                }
                                _ => {}
                            },
                        }
                    }
                })
            }))
            .await;

        let s_out_1 = s.clone();
        let r_out_1 = r.clone();
        down_track_arc
            .on_bind(Box::new(move || {
                let s_in = s_out_1.clone();
                let r_in = r_out_1.clone();

                Box::pin(async move {
                    tokio::spawn(async move {
                        s_in.send_stream_down_track_reports(r_in.stream_id()).await;
                    });
                })
            }))
            .await;
        log::info!("add_down_track 4..");
        s.add_down_track(recv.stream_id(), down_track_arc.clone())
            .await;
        log::info!("add_down_track 5..");
        recv.add_down_track(down_track_arc, self.config.simulcast.best_quality_first)
            .await;
        log::info!("add_down_track 6..");
        Ok(None)
    }

    async fn set_rtcp_writer(&self, writer: RtcpWriterFn) {
        let mut handler = self.rtcp_writer_handler.lock().await;
        *handler = Some(writer);
    }

    async fn send_rtcp(&self) {
        loop {
            let mut rtcp_receiver = self.rtcp_receiver_channel.lock().await;
            let mut stop_receiver = self.stop_receiver_channel.lock().await;
            tokio::select! {
              data = rtcp_receiver.recv() => {
                if let Some(val) = data{
                    if let Some(f) = &mut *self.rtcp_writer_handler.lock().await {
                        f(val);
                    }
                }
              }
              data = stop_receiver.recv() => {
                return ;
              }
            };
        }
    }
}

async fn delete_receiver(
    track: &String,
    ssrc: &u32,
    del_handler: Arc<Mutex<Option<OnDelReciverTrackFn>>>,
    receivers: Arc<Mutex<HashMap<String, Arc<dyn Receiver + Send + Sync>>>>,
    stats: Arc<Mutex<HashMap<u32, Stream>>>,
) {
    if let Some(f) = &mut *del_handler.lock().await {
        if let Some(track) = receivers.lock().await.get(track) {
            f(track.clone());
        }
    }
    receivers.lock().await.remove(track);
    stats.lock().await.remove(ssrc);
}
