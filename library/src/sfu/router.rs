use super::simulcast::SimulcastConfig;

use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;
pub struct RouterConfig {
    with_stats: bool,
    max_bandwidth: u64,
    max_packet_track: i32,
    audio_level_interval: i32,
    audio_level_threshold: u8,
    audio_level_filter: i32,
    simulcast: SimulcastConfig,
}

trait Router {
    fn id() -> String;
    fn add_receiver(receiver: RTCRtpReceiver);
}
