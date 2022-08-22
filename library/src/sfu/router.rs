use super::simulcast::SimulcastConfig;

use super::down_track::DownTrack;
use super::receiver::Receiver;
use super::receiver::WebRTCReceiver;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use super::subscriber::Subscriber;
use crate::twcc::twcc::Responder;
use anyhow::Result;
use async_trait::async_trait;
use rtcp::packet::Packet as RtcpPacket;
use rtcp::raw_packet::RawPacket;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;

use tokio::sync::{broadcast, mpsc, oneshot};

pub type RtcpDataSender = mpsc::UnboundedSender<Vec<Box<Arc<dyn RtcpPacket + Send + Sync>>>>;

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
    fn add_down_tracks(&mut self) -> Result<()>;

    // pub async fn write_rtcp(
    //     &self,
    //     pkts: &[Box<dyn rtcp::packet::Packet + Send + Sync>],
    // ) -> Result<usize>

    fn set_peer_connection(&mut self, pc: Arc<RTCPeerConnection>);
    fn add_down_track(
        &mut self,
        subscriber: Subscriber,
        receiver: Arc<dyn Receiver + Send + Sync>,
    ) -> Result<()>;
    async fn stop(&mut self);
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
    twcc: Option<Responder>,
    rtcp_channel: Arc<RtcpDataSender>,
    stop_channel: mpsc::Sender<()>,
    config: RouterConfig,
    session: Arc<Mutex<dyn Session + Send + Sync>>,
    receivers: HashMap<String, Arc<Mutex<dyn Receiver + Send + Sync>>>,
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
            twcc: None,
            rtcp_channel: Arc::new(s),
            stop_channel: sender,
            config,
            session,
            receivers: HashMap::new(),
        }
    }
}
#[async_trait]
impl Router for RouterLocal {
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

        match track.kind() {
            RTPCodecType::Audio => {
                if let Some(mut observer) = self.session.lock().await.audio_obserber() {
                    observer.add_stream(stream_id).await;
                }
            }
            RTPCodecType::Video => {
                if let Some(twcc) = &mut self.twcc {
                    let sender = Arc::clone(&self.rtcp_channel);
                    twcc.on_feedback(Box::new(
                        move |rtcp_packet: Arc<dyn RtcpPacket + Send + Sync>| {
                            let sender_in = Arc::clone(&sender);
                            Box::pin(async move {
                                let mut data = Vec::new();
                                data.push(Box::new(rtcp_packet));
                                sender_in.send(data);
                            })
                        },
                    ))
                    .await;
                } else {
                    self.twcc = Some(Responder::new(track.ssrc()));
                }
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

    fn add_down_tracks(&mut self) -> Result<()> {
        Ok(())
    }

    fn add_down_track(
        &mut self,
        subscriber: Subscriber,
        receiver: Arc<dyn Receiver + Send + Sync>,
    ) -> Result<()> {
        // subscriber.me.

        // let rv =  DownTrack::new(c, r, peer_id, mt)

        Ok(())
    }

    fn set_peer_connection(&mut self, pc: Arc<RTCPeerConnection>) {
        // Ok(());
    }
}
