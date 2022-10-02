use std::sync::Arc;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_remote::TrackRemote;

use super::down_track::DownTrack;
use super::sequencer::PacketMeta;
use super::simulcast;
use rtcp::packet::Packet as RtcpPacket;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;

use super::errors::Error;
use super::errors::Result;
use crate::buffer::buffer::Buffer;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
// use webrtc::error::Result;

use crate::stats::stream::Stream;
use std::sync::Weak;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::down_track::DownTrackType;
use async_trait::async_trait;
use tokio::sync::Mutex;

use super::down_track::DownTrackInfo;

use crate::buffer::errors::Error as SfuBufferError;
use std::any::Any;

use webrtc::error::Error as RTCError;

pub type RtcpDataReceiver = mpsc::UnboundedReceiver<Vec<Box<dyn RtcpPacket + Send + Sync>>>;

#[async_trait]
pub trait Receiver: Send + Sync {
    fn track_id(&self) -> String;
    fn stream_id(&self) -> String;
    fn codec(&self) -> RTCRtpCodecParameters;
    fn kind(&self) -> RTPCodecType;
    fn ssrc(&self, layer: usize) -> u32;
    fn set_track_meta(&mut self, track_id: String, stream_id: String);
    async fn add_up_track(&mut self, track: TrackRemote, buffer: Buffer, best_quality_first: bool);
    async fn add_down_track(&mut self, track: Arc<DownTrack>, best_quality_first: bool);
    async fn switch_down_track(&self, track: Arc<DownTrack>, layer: usize) -> Result<()>;
    fn get_bitrate(&self) -> Vec<u64>;
    fn get_max_temporal_layer(&self) -> Vec<i32>;
    fn retransmit_packets(&self, track: Arc<DownTrack>, packets: &[PacketMeta]) -> Result<()>;
    async fn delete_down_track(&mut self, layer: usize, id: String);
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
    buffers: [Option<Buffer>; 3],
    up_tracks: [Option<TrackRemote>; 3],
    stats: [Option<Stream>; 3],
    available: [AtomicBool; 3],
    down_tracks: [Arc<Mutex<Vec<Arc<DownTrack>>>>; 3],
    pending: [AtomicBool; 3],
    pending_tracks: [Arc<Mutex<Vec<Arc<DownTrack>>>>; 3],
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
            buffers: [None, None, None],
            up_tracks: [None, None, None],
            stats: [None, None, None],
            available: [
                AtomicBool::default(),
                AtomicBool::default(),
                AtomicBool::default(),
            ],
            down_tracks: [
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(Vec::new())),
            ],
            pending: [
                AtomicBool::default(),
                AtomicBool::default(),
                AtomicBool::default(),
            ],
            pending_tracks: [
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(Vec::new())),
            ],
            // ..Default::default()
        }
    }
}

#[async_trait]
impl Receiver for WebRTCReceiver {
    fn set_track_meta(&mut self, track_id: String, stream_id: String) {
        self.stream_id = stream_id;
        self.track_id = track_id;
    }

    fn stream_id(&self) -> String {
        self.stream_id.clone()
    }
    fn track_id(&self) -> String {
        self.track_id.clone()
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

    async fn add_up_track(&mut self, track: TrackRemote, buffer: Buffer, best_quality_first: bool) {
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
        self.buffers[layer] = Some(buffer);
        self.available[layer] = AtomicBool::new(true);

        let down_tracks_clone = self.down_tracks.clone();
        let sub_best_quality = |target_layer| async move {
            for l in 0..target_layer {
                let mut dts = down_tracks_clone[l].lock().await;
                if dts.len() == 0 {
                    continue;
                }
                for d in &mut *dts {
                    d.switch_spatial_layer(target_layer as i32, false).await;
                }
            }
        };

        let down_tracks_clone_2 = self.down_tracks.clone();
        let sub_lowest_quality = |target_layer: usize| async move {
            for l in (target_layer + 1..3).rev() {
                let mut dts = down_tracks_clone_2[l].lock().await;
                if dts.len() == 0 {
                    continue;
                }
                for d in &mut *dts {
                    d.switch_spatial_layer(target_layer as i32, false).await;
                }
            }
        };

        if self.is_simulcast {
            if best_quality_first && (self.available[2].load(Ordering::Relaxed) || layer == 2) {
                sub_best_quality(layer).await;
            } else if !best_quality_first
                && (self.available[0].load(Ordering::Relaxed) || layer == 0)
            {
                sub_lowest_quality(layer).await;
            }
        }

        // tokio::spawn(async move { self.write_rtp(layer) });
    }
    async fn add_down_track(&mut self, track: Arc<DownTrack>, best_quality_first: bool) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let mut layer = 0;

        if self.is_simulcast {
            for (idx, v) in self.available.iter().enumerate() {
                if v.load(Ordering::Relaxed) {
                    layer = idx;
                    if !best_quality_first {
                        break;
                    }
                }
            }
            if self.down_track_subscribed(layer, track.clone()).await {
                return;
            }
            track.set_initial_layers(layer as i32, 2);
            track.set_max_spatial_layer(2);
            track.set_max_temporal_layer(2);
            track.set_last_ssrc(self.ssrc(layer));
            track
                .set_track_type(DownTrackType::SimulcastDownTrack)
                .await;
            // track.payload =
        } else {
            if self.down_track_subscribed(layer, track.clone()).await {
                return;
            }
            track.set_initial_layers(0, 0);
            track.set_track_type(DownTrackType::SimpleDownTrack).await;
        }

        self.store_down_track(layer, track).await
    }
    async fn switch_down_track(&self, track: Arc<DownTrack>, layer: usize) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::ErrNoReceiverFound.into());
        }

        if self.available[layer].load(Ordering::Relaxed) {
            self.pending[layer].store(true, Ordering::Relaxed);
            self.pending_tracks[layer].lock().await.push(track);
        }
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
    async fn delete_down_track(&mut self, layer: usize, id: String) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }

        let mut down_tracks = self.down_tracks[layer].lock().await;
        let mut idx: usize = 0;
        for dt in &mut *down_tracks {
            //let mut dt_raw = dt.lock().await;
            if dt.id == id {
                dt.close().await;
                break;
            }
            idx = idx + 1;
        }

        down_tracks.remove(idx);
    }
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

impl WebRTCReceiver {
    async fn write_rtp(&mut self, layer: usize) -> Result<()> {
        // let pli = Vec::new();
        // let pkt = Box::new(PictureLossIndication {
        //     media_ssrc: self.ssrc(layer),
        //     sender_ssrc: rand::random::<u32>(),
        //     ..Default::default()
        // } as (dyn RtcpPacket + Send + Sync));

        // let pli = vec![Box::new(PictureLossIndication {
        //     sender_ssrc: rand::random::<u32>(),
        //     media_ssrc: self.ssrc(layer),
        // })];

        loop {
            if let Some(buffer) = &mut self.buffers[layer] {
                match buffer.read_extended().await {
                    Ok(pkt) => {
                        if self.is_simulcast && self.pending[layer].load(Ordering::Relaxed) {
                            if pkt.key_frame {
                                //use tmp_val here just to skip the build error
                                let mut tmp_val = Vec::new();
                                {
                                    let mut dts = self.pending_tracks[layer].lock().await;
                                    for dt in &mut *dts {
                                        //let dt_raw = dt.lock().await;
                                        let current_spatial_layer =
                                            dt.current_spatial_layer() as usize;
                                        let id = dt.id.clone();
                                        tmp_val.push((current_spatial_layer, id, dt.clone()));
                                    }
                                }
                                for v in tmp_val {
                                    self.delete_down_track(v.0, v.1).await;
                                    let dt = v.2;
                                    self.store_down_track(layer, dt.clone()).await;
                                    dt.switch_spatial_layer_done(layer as i32);
                                }
                                self.pending_tracks[layer].lock().await.clear();
                                self.pending[layer].store(false, Ordering::Relaxed);
                            } else {
                                self.send_rtcp(vec![Box::new(PictureLossIndication {
                                    sender_ssrc: rand::random::<u32>(),
                                    media_ssrc: self.ssrc(layer),
                                })]);
                            }
                        }

                        let mut delete_down_track_params = Vec::new();
                        {
                            let mut dts = self.down_tracks[layer].lock().await;
                            for dt in &mut *dts {
                                //let mut dt_value = dt.lock().await;
                                if let Err(err) = dt.write_rtp(pkt.clone(), layer).await {
                                    match err {
                                        RTCError::ErrClosedPipe
                                        | RTCError::ErrDataChannelNotOpen
                                        | RTCError::ErrConnectionClosed => {
                                            delete_down_track_params.push((layer, dt.id.clone()));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        for (layer, id) in delete_down_track_params {
                            self.delete_down_track(layer, id).await;
                        }
                    }
                    Err(e) => match e {
                        SfuBufferError::ErrIOEof => {}
                        _ => {}
                    },
                }
            }
        }
    }

    async fn down_track_subscribed(&self, layer: usize, dt: Arc<DownTrack>) -> bool {
        let down_tracks = self.down_tracks[layer].lock().await;
        for down_track in &*down_tracks {
            if **down_track == *dt {
                return true;
            }
        }

        true
    }

    async fn store_down_track(&self, layer: usize, dt: Arc<DownTrack>) {
        let dts = &mut self.down_tracks[layer].lock().await;
        dts.push(dt);
    }
}
