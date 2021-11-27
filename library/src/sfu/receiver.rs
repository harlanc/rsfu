use webrtc::media::rtp::rtp_codec::RTCRtpCodecParameters;
use webrtc::media::rtp::rtp_codec::RTPCodecType;
use webrtc::media::track::track_remote::TrackRemote;

use crate::buffer::buffer::Buffer;
pub trait Receiver {
    fn track_id(&self) -> String;
    fn stream_id(&self) -> String;
    fn codec(&self) -> RTCRtpCodecParameters;
    fn kind(&self) -> RTPCodecType;
    fn ssrc(&self, layer: u16) -> u32;
    fn set_track_meta(&self, track_id: String, stream_id: String);
    fn add_up_track(&self, track: TrackRemote, buffer: Buffer, best_quality_first: bool);
    fn add_download_track(&self);
}
