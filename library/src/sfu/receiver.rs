use std::sync::Arc;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_remote::TrackRemote;

use super::down_track::DownTrack;
use super::sequencer::PacketMeta;
use rtcp::packet::Packet as RtcpPacket;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;

use crate::buffer::buffer::Buffer;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use crate::stats::stream::Stream;
use tokio::sync::{broadcast, mpsc, oneshot};

pub type RtcpDataReceiver = mpsc::UnboundedReceiver<Vec<Box<dyn RtcpPacket>>>;
pub trait Receiver {
    fn track_id(&self) -> String;
    fn stream_id(&self) -> String;
    fn codec(&self) -> RTCRtpCodecParameters;
    fn kind(&self) -> RTPCodecType;
    fn ssrc(&self, layer: u16) -> u32;
    fn set_track_meta(&mut self, track_id: String, stream_id: String);
    fn add_up_track(&mut self, track: TrackRemote, buffer: Buffer, best_quality_first: bool);
    fn add_down_track(&mut self, track: Arc<DownTrack>, best_quality_first: bool);
    fn switch_down_track(&self, track: Arc<DownTrack>, layer: u8);
    fn get_bitrate() -> [u64; 3];
    fn get_max_temporal_layer() -> [i32; 3];
    fn retransmit_packets(track: Arc<DownTrack>, packets: &[PacketMeta]) -> Result<()>;
    fn delete_down_track(layer: u8, id: String);
    fn on_close_handler(func: fn());
    fn send_rtcp(p: Vec<Box<dyn RtcpPacket>>);
    fn set_rtcp_channel();
    fn get_sender_report_time(layer: u8) -> (u32, u64);
}
#[derive(Default)]
pub struct WebRTCReceiver {
    peer_id: String,
    track_id: String,
    stream_id: String,

    kind: RTPCodecType,
    closed: AtomicBool,
    bandwidth: u64,
    last_pli: i64,
    stream: String,
    receiver: RTCRtpReceiver,
    codec: RTCRtpCodecParameters,
    rtcp_channel: RtcpDataReceiver,
    buffers: [Buffer; 3],
    up_tracks: [Option<TrackRemote>; 3],
    stats: [Stream; 3],
    available: [AtomicBool; 3],
    down_tracks: [Vec<DownTrack>; 3],
    pending: [AtomicBool; 3],
    pending_tracks: [Vec<DownTrack>; 3],
    is_simulcast: bool,
    on_close_handler: fn(),
}

impl WebRTCReceiver {
    pub fn new(receiver: RTCRtpReceiver, track: TrackRemote, pid: String) -> Self {
        Self {
            peer_id: pid,
            receiver,
            track_id: track.id(),
            stream_id: track.stream_id(),
            codec: track.codec(),
            kind: track.kind(),
            is_simulcast: track.rid().len() > 0,
            ..Default::default()
        }
    }
}

impl Receiver for WebRTCReceiver {
    fn track_id(&self) -> String {
        self.track_id
    }

    fn stream_id(&self) -> String {
        self.stream_id
    }
    fn codec(&self) -> RTCRtpCodecParameters {
        self.codec
    }
    fn kind(&self) -> RTPCodecType {
        self.kind
    }
    fn ssrc(&self, layer: u16) -> u32 {
        if layer < 3 {
            if let Some(track) = self.up_tracks[layer] {
                return track.ssrc();
            }
        }

        return 0;
    }

    fn set_track_meta(&mut self, track_id: String, stream_id: String) {
        self.stream_id = stream_id;
        self.track_id = track_id;
    }
    fn add_up_track(&mut self, track: TrackRemote, buffer: Buffer, best_quality_first: bool) {
        if self.closed.load(Ordering::Release) {
            return;
        }

        let mut layer: u8;

        match track.rid() {}
    }
    fn add_down_track(&mut self, track: Arc<DownTrack>, best_quality_first: bool) {}
    fn switch_down_track(&self, track: Arc<DownTrack>, layer: u8) {}
}
