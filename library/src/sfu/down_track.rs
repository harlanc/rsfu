// DownTrackType determines the type of track
//type DownTrackType =  u16;

use super::sequencer::{self, AtomicSequencer};
use super::simulcast::SimulcastTrackHelpers;
use atomic::Atomic;
use rtp::extension::audio_level_extension::AudioLevelExtension;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

use super::receiver::Receiver;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::Mutex;
use webrtc::media::rtp::rtp_codec::RTCRtpCodecCapability;
use webrtc::media::rtp::rtp_transceiver::RTCRtpTransceiver;

use webrtc::media::rtp::rtp_codec::RTCRtpCodecParameters;
use webrtc::media::track::track_local::{TrackLocal, TrackLocalContext, TrackLocalWriter};

pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub type OnBindFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

enum DownTrackType {
    SimpleDownTrack,
    SimulcastDownTrack,
}

pub struct DownTrack {
    id: String,
    peer_id: String,
    bound: AtomicBool,
    mime: String,
    ssrc: u32,
    stream_id: String,
    max_track: i32,
    payload_type: u8,
    sequencer: AtomicSequencer,
    track_type: DownTrackType,
    pub payload: Vec<u8>,

    current_spatial_layer: i32,
    target_spatial_layer: i32,
    pub temporal_layer: AtomicI32,

    enabled: AtomicBool,
    re_sync: AtomicBool,
    sn_offset: u16,
    ts_offset: u32,
    last_ssrc: u32,
    last_sn: u16,
    last_ts: u32,

    pub simulcast: SimulcastTrackHelpers,
    max_spatial_layer: i32,
    max_temporal_layer: i32,

    codec: RTCRtpCodecCapability,
    receiver: Arc<dyn Receiver + Send + Sync>,
    transceiver: Option<RTCRtpTransceiver>,
    write_stream: Option<Arc<dyn TrackLocalWriter + Send + Sync>>, //TrackLocalWriter,
    on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,
    on_bind_handler: Arc<Mutex<Option<OnBindFn>>>,

    close_once: Once,

    octet_count: u32,
    packet_count: u32,
    max_packet_ts: u32,
}

impl DownTrack {
    fn new(
        c: RTCRtpCodecCapability,
        r: Arc<dyn Receiver + Send + Sync>,
        peer_id: String,
        mt: i32,
    ) -> Self {
        Self {
            id: String::from(""),
            peer_id: String::from(""),
            bound: AtomicBool::new(false),
            mime: String::from(""),
            ssrc: 0,
            stream_id: String::from(""),
            max_track: 0,
            payload_type: 0,
            sequencer: AtomicSequencer::new(0),
            track_type: DownTrackType::SimpleDownTrack,
            payload: Vec::new(),

            current_spatial_layer: 0,
            target_spatial_layer: 0,
            temporal_layer: AtomicI32::new(0),

            enabled: AtomicBool::new(false),
            re_sync: AtomicBool::new(false),
            sn_offset: 0,
            ts_offset: 0,
            last_ssrc: 0,
            last_sn: 0,
            last_ts: 0,

            simulcast: SimulcastTrackHelpers::new(),
            max_spatial_layer: 0,
            max_temporal_layer: 0,

            codec: c,
            receiver: r,
            transceiver: None,
            write_stream: None,
            on_close_handler: Arc::default(),
            on_bind_handler: Arc::default(),

            close_once: Once::new(),

            octet_count: 0,
            packet_count: 0,
            max_packet_ts: 0,
        }
    }

    // fn bind(&mut self, t: TrackLocalContext) -> Result<RTCRtpCodecParameters> {

    //     let parameters =

    // }
}
