use super::simulcast::SimulcastConfig;

use super::receiver::Receiver;
use anyhow::Result;
use async_trait::async_trait;
use rtcp::packet::Packet as RtcpPacket;
use std::sync::Arc;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;

#[async_trait]
pub trait Router {
    // ID() string
    async fn id() -> String;
    // AddReceiver(receiver *webrtc.RTPReceiver, track *webrtc.TrackRemote, trackID, streamID string) (Receiver, bool)
    async fn add_receiver(receiver: RTCRtpReceiver) -> Result<Arc<dyn Receiver + Send + Sync>>;
    // AddDownTracks(s *Subscriber, r Receiver) error
    async fn add_down_tracks() -> Result<()>;
    // SetRTCPWriter(func([]rtcp.Packet) error)
    async fn set_rtcp_writer(writer: fn(Vec<Box<dyn RtcpPacket + Send + Sync>>) -> Result<()>);
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
