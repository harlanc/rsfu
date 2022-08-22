use std::sync::Arc;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_remote::TrackRemote;

use super::down_track::DownTrack;
use super::sequencer::PacketMeta;
use super::simulcast;
use rtcp::packet::Packet as RtcpPacket;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;

use crate::buffer::buffer::Buffer;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use webrtc::error::{Error, Result};

use crate::stats::stream::Stream;
use std::sync::Weak;
use tokio::sync::{broadcast, mpsc, oneshot};

use std::any::Any;

pub type RtcpDataReceiver = mpsc::UnboundedReceiver<Vec<Box<dyn RtcpPacket + Send + Sync>>>;
pub trait Receiver: Send + Sync {
    fn track_id(&self) -> String;
    fn stream_id(&self) -> String;
    fn codec(&self) -> RTCRtpCodecParameters;
    fn kind(&self) -> RTPCodecType;
    fn ssrc(&self, layer: usize) -> u32;
    fn set_track_meta(&mut self, track_id: String, stream_id: String);
    fn add_up_track(&mut self, track: TrackRemote, buffer: Buffer, best_quality_first: bool);
    fn add_down_track(&mut self, track: Arc<DownTrack>, best_quality_first: bool);
    fn switch_down_track(&self, track: Weak<DownTrack>, layer: u8) -> Result<()>;
    fn get_bitrate(&self) -> Vec<u64>;
    fn get_max_temporal_layer(&self) -> Vec<i32>;
    fn retransmit_packets(&self, track: Arc<DownTrack>, packets: &[PacketMeta]) -> Result<()>;
    fn delete_down_track(&self, layer: u8, id: String);
    fn on_close_handler(&self, func: fn());
    fn send_rtcp(&self, p: Vec<Box<dyn RtcpPacket + Send + Sync>>);
    fn set_rtcp_channel(&self);
    fn get_sender_report_time(&self, layer: u8) -> (u32, u64);
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

pub struct WebRTCReceiver {
    peer_id: String,
    track_id: String,
    stream_id: String,

    kind: RTPCodecType,
    closed: AtomicBool,
    bandwidth: u64,
    last_pli: i64,
    stream: String,
    pub receiver: Arc<RTCRtpReceiver>,
    codec: RTCRtpCodecParameters,
    rtcp_channel: RtcpDataReceiver,
    buffers: Vec<Buffer>,
    up_tracks: Vec<Option<TrackRemote>>,
    stats: Vec<Stream>,
    available: Vec<AtomicBool>,
    down_tracks: Vec<Option<Vec<DownTrack>>>,
    pending: Vec<AtomicBool>,
    pending_tracks: Vec<Option<Vec<DownTrack>>>,
    is_simulcast: bool,
    //on_close_handler: fn(),
}

impl WebRTCReceiver {
    pub async fn new(receiver: Arc<RTCRtpReceiver>, track: Arc<TrackRemote>, pid: String) -> Self {
        let (s, r) = mpsc::unbounded_channel();
        Self {
            peer_id: pid,
            receiver: receiver,
            track_id: track.id().await,
            stream_id: track.stream_id().await,
            codec: track.codec().await,
            kind: track.kind(),
            is_simulcast: track.rid().len() > 0,
            closed: AtomicBool::default(),
            bandwidth: 0,
            last_pli: 0,
            stream: String::default(),
            rtcp_channel: r,
            buffers: Vec::new(),
            up_tracks: Vec::new(),
            stats: Vec::new(),
            available: Vec::new(),
            down_tracks: Vec::new(),
            pending: Vec::new(),
            pending_tracks: Vec::new(),
            // ..Default::default()
        }
    }
}

impl Receiver for WebRTCReceiver {
    fn track_id(&self) -> String {
        self.track_id.clone()
    }

    fn stream_id(&self) -> String {
        self.stream_id.clone()
    }
    fn codec(&self) -> RTCRtpCodecParameters {
        self.codec.clone()
    }
    fn kind(&self) -> RTPCodecType {
        self.kind
    }
    fn ssrc(&self, layer: usize) -> u32 {
        if layer < 3 {
            if let Some(track) = &self.up_tracks[layer] {
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
        if self.closed.load(Ordering::Acquire) {
            return;
        }

        let mut layer: usize;

        match track.rid() {
            simulcast::FULL_RESOLUTION => {
                layer = 2;
            }

            simulcast::HALF_RESOLUTION => {
                layer = 1;
            }

            simulcast::QUARTER_RESOLUTION => {
                layer = 0;
            }

            _ => {
                layer = 0;
            }
        }

        self.up_tracks[layer] = Some(track);
        self.buffers[layer] = buffer;
        self.available[layer] = AtomicBool::new(true);

        if self.is_simulcast {
            if best_quality_first && (self.available[2].load(Ordering::Relaxed) || layer == 2) {
                for l in 0..layer {
                    // if let Some(dts) = self.down_tracks[layer] {
                    //     for dt in dts {
                    //         //  dt
                    //     }
                    // }

                    // for dt in dts {}
                }
            }
        }
    }
    fn add_down_track(&mut self, track: Arc<DownTrack>, best_quality_first: bool) {}
    fn switch_down_track(&self, track: Weak<DownTrack>, layer: u8) -> Result<()> {
        Ok(())
    }

    fn get_bitrate(&self) -> Vec<u64> {
        Vec::new()
    }
    fn get_max_temporal_layer(&self) -> Vec<i32> {
        Vec::new()
    }
    fn retransmit_packets(&self, track: Arc<DownTrack>, packets: &[PacketMeta]) -> Result<()> {
        Ok(())
    }
    fn delete_down_track(&self, layer: u8, id: String) {}
    fn on_close_handler(&self, func: fn()) {}
    fn send_rtcp(&self, p: Vec<Box<dyn RtcpPacket + Send + Sync>>) {}
    fn set_rtcp_channel(&self) {}
    fn get_sender_report_time(&self, layer: u8) -> (u32, u64) {
        (0, 0)
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}
