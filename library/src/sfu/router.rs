use super::simulcast::SimulcastConfig;

use super::receiver::Receiver;
use crate::twcc::twcc::Responder;
use anyhow::Result;
use async_trait::async_trait;
use rtcp::packet::Packet as RtcpPacket;
use std::sync::Arc;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;

use tokio::sync::{broadcast, mpsc, oneshot};

pub type RtcpDataReceiver = mpsc::UnboundedReceiver<Vec<Box<dyn RtcpPacket>>>;

#[async_trait]
pub trait Router {
    // ID() string
    async fn id(&self) -> String;
    // AddReceiver(receiver *webrtc.RTPReceiver, track *webrtc.TrackRemote, trackID, streamID string) (Receiver, bool)
    async fn add_receiver(
        &mut self,
        receiver: RTCRtpReceiver,
    ) -> Result<Arc<dyn Receiver + Send + Sync>>;
    // AddDownTracks(s *Subscriber, r Receiver) error
    async fn add_down_tracks(&mut self) -> Result<()>;
    // SetRTCPWriter(func([]rtcp.Packet) error)
    async fn set_rtcp_writer(
        &mut self,
        writer: fn(Vec<Box<dyn RtcpPacket + Send + Sync>>) -> Result<()>,
    );
    // AddDownTrack(s *Subscriber, r Receiver) (*DownTrack, error)
    // Stop()
}

#[derive(Default, Clone)]
pub struct RouterConfig {
    pub(super) with_stats: bool,
    max_bandwidth: u64,
    max_packet_track: i32,
    audio_level_interval: i32,
    audio_level_threshold: u8,
    audio_level_filter: i32,
    simulcast: SimulcastConfig,
}

struct RouterImpl {
    id: String,
    twcc: Responder,
    rtcp_channel: RtcpDataReceiver,
    stop_channel: Option<mpsc::Sender<()>>,
    config: RouterConfig,
}
#[async_trait]
impl Router for RouterImpl {
    async fn id(&self) -> String {
        self.id
    }
    // AddReceiver(receiver *webrtc.RTPReceiver, track *webrtc.TrackRemote, trackID, streamID string) (Receiver, bool)
    async fn add_receiver(
        &mut self,
        receiver: RTCRtpReceiver,
    ) -> Result<Arc<dyn Receiver + Send + Sync>> {
    }
    // AddDownTracks(s *Subscriber, r Receiver) error
    async fn add_down_tracks(&mut self) -> Result<()> {
        Ok(())
    }
    // SetRTCPWriter(func([]rtcp.Packet) error)
    async fn set_rtcp_writer(
        &mut self,
        writer: fn(Vec<Box<dyn RtcpPacket + Send + Sync>>) -> Result<()>,
    ) {
        Ok(())
    }
}
