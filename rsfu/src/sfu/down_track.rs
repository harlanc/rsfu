// DownTrackType determines the type of track
//type DownTrackType =  u16;
use super::errors::Error;
use super::errors::Result;

use super::helpers;
use super::receiver::Receiver;
use super::simulcast::SimulcastTrackHelpers;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use webrtc::error::Error as WEBRTCError;
use webrtc::error::Result as RTCResult;

use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Once;
use std::time::SystemTime;
use tokio::sync::Mutex;
use webrtc::rtcp::sender_report::SenderReport;
use webrtc::rtcp::source_description::SdesType;
use webrtc::rtcp::source_description::SourceDescriptionChunk;
use webrtc::rtcp::source_description::SourceDescriptionItem;

use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::RTCRtpTransceiver;

use async_trait::async_trait;
use tokio::time::Duration;

use crate::buffer::buffer::ExtPacket;

use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::{TrackLocal, TrackLocalContext};

use super::down_track_internal::DownTrackInternal;
use std::any::Any;

pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub type OnBindFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

#[derive(Default, Clone)]
pub enum DownTrackType {
    #[default]
    SimpleDownTrack,
    SimulcastDownTrack,
}

#[derive(Default, Clone)]
pub struct DownTrackInfo {
    pub layer: i32,
    pub last_ssrc: u32,
    pub track_type: DownTrackType,
    pub payload: Vec<u8>,
}

pub struct DownTrack {
    peer_id: String,
    track_type: Mutex<DownTrackType>,
    pub payload: Vec<u8>,

    current_spatial_layer: AtomicI32,
    target_spatial_layer: AtomicI32,
    pub temporal_layer: AtomicI32,

    sn_offset: Mutex<u16>,
    ts_offset: Mutex<u32>,

    last_sn: Mutex<u16>,
    last_ts: Mutex<u32>,

    pub simulcast: Arc<Mutex<SimulcastTrackHelpers>>,
    max_spatial_layer: AtomicI32,
    max_temporal_layer: AtomicI32,
    pub transceiver: Option<Arc<RTCRtpTransceiver>>,
    pub on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,

    #[allow(dead_code)]
    close_once: Once,
    octet_count: AtomicU32,
    #[allow(dead_code)]
    packet_count: AtomicU32,
    #[allow(dead_code)]
    max_packet_ts: u32,

    down_track_local: Arc<DownTrackInternal>,
}

impl PartialEq for DownTrack {
    fn eq(&self, other: &Self) -> bool {
        return (self.peer_id == other.peer_id)
            && (self.down_track_local == other.down_track_local);
    }
}

impl DownTrack {
    pub(super) async fn new(
        c: RTCRtpCodecCapability,
        r: Arc<dyn Receiver + Send + Sync>,
        peer_id: String,
        mt: i32,
    ) -> Self {
        Self {
            peer_id: peer_id,
            track_type: Mutex::new(DownTrackType::SimpleDownTrack),
            payload: Vec::new(),
            current_spatial_layer: AtomicI32::new(0),
            target_spatial_layer: AtomicI32::new(0),
            temporal_layer: AtomicI32::new(0),
            sn_offset: Mutex::new(0),
            ts_offset: Mutex::new(0),
            last_sn: Mutex::new(0),
            last_ts: Mutex::new(0),
            simulcast: Arc::new(Mutex::new(SimulcastTrackHelpers::new())),
            max_spatial_layer: AtomicI32::new(0),
            max_temporal_layer: AtomicI32::new(0),
            transceiver: None,
            on_close_handler: Arc::default(),
            close_once: Once::new(),
            octet_count: AtomicU32::new(0),
            packet_count: AtomicU32::new(0),
            max_packet_ts: 0,
            down_track_local: Arc::new(DownTrackInternal::new(c, r, mt).await),
        }
    }

    pub(super) fn new_track_local(peer_id: String, track: Arc<DownTrackInternal>) -> Self {
        Self {
            peer_id: peer_id,
            track_type: Mutex::new(DownTrackType::SimpleDownTrack),
            payload: Vec::new(),

            current_spatial_layer: AtomicI32::new(0),
            target_spatial_layer: AtomicI32::new(0),
            temporal_layer: AtomicI32::new(0),

            sn_offset: Mutex::new(0),
            ts_offset: Mutex::new(0),

            last_sn: Mutex::new(0),
            last_ts: Mutex::new(0),

            simulcast: Arc::new(Mutex::new(SimulcastTrackHelpers::new())),
            max_spatial_layer: AtomicI32::new(0),
            max_temporal_layer: AtomicI32::new(0),

            transceiver: None,
            on_close_handler: Arc::default(),
            close_once: Once::new(),

            octet_count: AtomicU32::new(0),
            packet_count: AtomicU32::new(0),
            max_packet_ts: 0,
            down_track_local: track,
        }
    }

    #[allow(dead_code)]
    fn codec(&self) -> RTCRtpCodecCapability {
        self.down_track_local.codec.clone()
    }
    #[allow(dead_code)]
    async fn stop(&self) -> Result<()> {
        if let Some(transceiver) = &self.transceiver {
            transceiver.stop().await?;
            return Ok(());
        }

        Err(Error::ErrWebRTC(WEBRTCError::new(String::from(
            "transceiver not exists",
        ))))
    }

    pub async fn ssrc(&self) -> u32 {
        let ssrc = self.down_track_local.ssrc.lock().await;
        *ssrc
    }

    pub async fn payload_type(&self) -> u8 {
        *self.down_track_local.payload_type.lock().await
    }

    pub async fn mime(&self) -> String {
        let mime = self.down_track_local.mime.lock().await;
        mime.clone()
    }

    pub fn set_transceiver(&mut self, transceiver: Arc<RTCRtpTransceiver>) {
        self.transceiver = Some(transceiver)
    }

    pub fn set_max_spatial_layer(&self, val: i32) {
        self.max_spatial_layer.store(val, Ordering::Release);
    }

    pub fn set_max_temporal_layer(&self, val: i32) {
        self.max_spatial_layer.store(val, Ordering::Release);
    }

    pub fn set_last_ssrc(&self, val: u32) {
        self.down_track_local
            .last_ssrc
            .store(val, Ordering::Release);
    }

    pub async fn set_track_type(&self, track_type: DownTrackType) {
        *self.track_type.lock().await = track_type;
    }

    pub async fn write_rtp(&self, pkt: ExtPacket, layer: usize) -> Result<()> {
        //let cur_payload_type = self.down_track_local.payload_type.lock().await.clone();

        if !self.down_track_local.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        if !self.bound() {
            return Ok(());
        }

        match *self.track_type.lock().await {
            DownTrackType::SimpleDownTrack => {
                return self.write_simple_rtp(pkt).await;
            }
            DownTrackType::SimulcastDownTrack => {
                return self.write_simulcast_rtp(pkt, layer as i32).await;
            }
        }
    }

    pub async fn write_raw_rtp(&self, pkt: rtp::packet::Packet) -> Result<()> {
        let write_stream_val = self.down_track_local.write_stream.lock().await;
        if let Some(write_stream) = &*write_stream_val {
            write_stream.write_rtp(&pkt).await?;
        }

        Ok(())
    }

    fn enabled(&self) -> bool {
        self.down_track_local.enabled.load(Ordering::Relaxed)
    }

    pub fn mute(&self, val: bool) {
        if self.enabled() != val {
            return;
        }
        self.down_track_local.enabled.store(!val, Ordering::Relaxed);
        if val {
            self.down_track_local.re_sync.store(val, Ordering::Relaxed);
        }
    }

    pub async fn close(&self) {
        // self.close_once.call_once(|| {
        let mut handler = self.on_close_handler.lock().await;
        if let Some(f) = &mut *handler {
            f().await;
        }
        // });
    }

    pub fn set_initial_layers(&self, spatial_layer: i32, temporal_layer: i32) {
        self.current_spatial_layer
            .store(spatial_layer, Ordering::Relaxed);
        self.target_spatial_layer
            .store(spatial_layer, Ordering::Relaxed);
        self.temporal_layer
            .store(temporal_layer << 16 | temporal_layer, Ordering::Relaxed);
    }

    pub fn current_spatial_layer(&self) -> i32 {
        self.current_spatial_layer.load(Ordering::Relaxed)
    }

    pub async fn switch_spatial_layer(
        self: &Arc<Self>,
        target_layer: i32,
        set_as_max: bool,
    ) -> Result<()> {
        match *self.track_type.lock().await {
            DownTrackType::SimulcastDownTrack => {
                let csl = self.current_spatial_layer.load(Ordering::Relaxed);
                if csl != self.target_spatial_layer.load(Ordering::Relaxed) || csl == target_layer {
                    //return Err(WEBRTCError::new(String::from("error spatial layer busy..")));
                    //return Err(ErrWebRTC::)

                    return Err(Error::ErrWebRTC(
                        WEBRTCError::new(String::from("error spatial layer busy..")).into(),
                    ));
                }
                let receiver = &self.down_track_local.receiver;
                match receiver
                    .switch_down_track(self.clone(), target_layer as usize)
                    .await
                {
                    Ok(_) => {
                        self.target_spatial_layer
                            .store(target_layer, Ordering::Relaxed);
                        if set_as_max {
                            self.max_spatial_layer
                                .store(target_layer, Ordering::Relaxed);
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }
            _ => {}
        }

        Err(Error::ErrWebRTC(WEBRTCError::new(String::from(
            "Error spatial not supported.",
        ))))
    }

    pub fn switch_spatial_layer_done(&self, layer: i32) {
        self.current_spatial_layer.store(layer, Ordering::Relaxed);
    }
    #[allow(dead_code)]
    async fn untrack_layers_change(self: &Arc<Self>, available_layers: &[u16]) -> Result<i64> {
        match *self.track_type.lock().await {
            DownTrackType::SimpleDownTrack => {
                let current_layer = self.current_spatial_layer.load(Ordering::Relaxed) as u16;
                let max_layer = self.max_spatial_layer.load(Ordering::Relaxed) as u16;

                let mut min_found: u16 = 0;
                let mut max_found: u16 = 0;
                let mut layer_found: bool = false;

                for target in available_layers.to_vec() {
                    if target <= max_layer {
                        if target > max_found {
                            max_found = target;
                            layer_found = true;
                        }
                    } else {
                        if min_found > target {
                            min_found = target;
                        }
                    }
                }

                let target_layer;
                if layer_found {
                    target_layer = max_found;
                } else {
                    target_layer = min_found;
                }

                if current_layer != target_layer {
                    if let Err(_) = self.switch_spatial_layer(target_layer as i32, false).await {
                        return Ok(target_layer as i64);
                    }
                }

                return Ok(target_layer as i64);
            }
            _ => {}
        }

        Err(Error::ErrWebRTC(WEBRTCError::new(format!(
            "downtrack {} does not support simulcast",
            self.down_track_local.id
        ))))
    }

    pub async fn switch_temporal_layer(&self, target_layer: i32, set_as_max: bool) {
        match *self.track_type.lock().await {
            DownTrackType::SimulcastDownTrack => {
                let layer = self.temporal_layer.load(Ordering::Relaxed);
                let current_layer = layer as u16;
                let current_target_layer = (layer >> 16) as u16;

                if current_layer != current_target_layer {
                    return;
                }

                self.temporal_layer.store(
                    (target_layer << 16) | (current_layer as i32),
                    Ordering::Relaxed,
                );

                if set_as_max {
                    self.max_temporal_layer
                        .store(target_layer, Ordering::Relaxed);
                }
            }

            _ => {}
        }
    }

    pub async fn on_close_handler(&self, f: OnCloseFn) {
        let mut handler = self.on_close_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_bind(&self, f: OnBindFn) {
        self.down_track_local.on_bind(f).await
    }

    pub async fn create_source_description_chunks(&self) -> Option<Vec<SourceDescriptionChunk>> {
        if !self.bound() {
            return None;
        }

        let mid = self.transceiver.as_ref().unwrap().mid().await;
        let ssrc = self.down_track_local.ssrc.lock().await.clone();

        Some(vec![
            SourceDescriptionChunk {
                source: ssrc,
                items: vec![SourceDescriptionItem {
                    sdes_type: SdesType::SdesCname,
                    text: Bytes::copy_from_slice(self.down_track_local.stream_id.as_bytes()),
                }],
            },
            SourceDescriptionChunk {
                source: ssrc,
                items: vec![SourceDescriptionItem {
                    sdes_type: SdesType::SdesCname,
                    text: Bytes::copy_from_slice(mid.as_bytes()),
                }],
            },
        ])
    }

    pub async fn create_sender_report(&self) -> Option<SenderReport> {
        if !self.bound() {
            return None;
        }

        let receiver = &self.down_track_local.receiver;
        let (sr_rtp, _sr_ntp) = receiver
            .get_sender_report_time(self.current_spatial_layer.load(Ordering::Relaxed) as usize)
            .await;

        if sr_rtp == 0 {
            return None;
        }

        let now = SystemTime::now();
        let now_ntp = helpers::to_ntp_time(now);

        let _clock_rate = self.down_track_local.codec.clock_rate;

        //todo
        let diff: u32 = 0;
        // if diff < 0 {
        //     diff = 0;
        // }

        let (octets, packets) = self.get_sr_status();

        let ssrc = self.down_track_local.ssrc.lock().await.clone();

        Some(SenderReport {
            ssrc: ssrc,
            ntp_time: u64::from(now_ntp),
            rtp_time: sr_rtp + diff,
            packet_count: packets,
            octet_count: octets,
            ..Default::default()
        })
    }

    pub fn update_stats(&self, packet_len: u32) {
        self.octet_count.store(packet_len, Ordering::Relaxed);
        self.packet_count.store(1, Ordering::Relaxed);
    }

    async fn write_simple_rtp(&self, packet: ExtPacket) -> Result<()> {
        let cur_payload_type = self.down_track_local.payload_type.lock().await.clone();
        let mut ext_packet = packet.clone();
        let ssrc = self.down_track_local.ssrc.lock().await.clone();

        if self.down_track_local.re_sync.load(Ordering::Relaxed) {
            match self.down_track_local.kind() {
                RTPCodecType::Video => {
                    if !ext_packet.key_frame {
                        let receiver = &self.down_track_local.receiver;
                        log::trace!(
                            "send PLI ssrc:{}, media_ssrc:{}",
                            ssrc,
                            ext_packet.packet.header.ssrc
                        );
                        receiver.send_rtcp(vec![Box::new(PictureLossIndication {
                            sender_ssrc: ssrc,
                            media_ssrc: ext_packet.packet.header.ssrc,
                        })])?;

                        return Ok(());
                    }
                }
                _ => {}
            }

            if *self.last_sn.lock().await != 0 {
                let mut sn_offset = self.sn_offset.lock().await;
                *sn_offset =
                    ext_packet.packet.header.sequence_number - *self.last_sn.lock().await - 1;
                let mut ts_offset = self.ts_offset.lock().await;
                *ts_offset = ext_packet.packet.header.timestamp - *self.last_ts.lock().await - 1
            }

            self.down_track_local
                .last_ssrc
                .store(ext_packet.packet.header.ssrc, Ordering::Relaxed);

            self.down_track_local
                .re_sync
                .store(false, Ordering::Relaxed);
        }
        self.update_stats(ext_packet.packet.payload.len() as u32);

        let new_sn = ext_packet.packet.header.sequence_number - *self.sn_offset.lock().await;
        let ts_offset = self.ts_offset.lock().await;
        let new_ts = ext_packet.packet.header.timestamp - *ts_offset;
        let mut sequencer = self.down_track_local.sequencer.lock().await;
        sequencer
            .push(
                ext_packet.packet.header.sequence_number,
                new_sn,
                new_ts,
                0,
                ext_packet.head,
            )
            .await;

        if ext_packet.head {
            let mut last_sn = self.last_sn.lock().await;
            *last_sn = new_sn;
            let mut last_ts = self.last_ts.lock().await;
            *last_ts = new_ts;
        }
        let header = &mut ext_packet.packet.header;
        header.payload_type = cur_payload_type;
        header.timestamp = new_ts;
        header.sequence_number = new_sn;
        header.ssrc = ssrc;

        let write_stream_val = self.down_track_local.write_stream.lock().await;
        if let Some(write_stream) = &*write_stream_val {
            let size = write_stream.write_rtp(&ext_packet.packet).await?;
            log::trace!("write_simple_rtp size:..{}", size);
        }

        Ok(())
    }

    async fn write_simulcast_rtp(&self, ext_packet: ExtPacket, layer: i32) -> Result<()> {
        let re_sync = self.down_track_local.re_sync.load(Ordering::Relaxed);
        let csl = self.current_spatial_layer();

        if csl != layer {
            return Ok(());
        }

        let ssrc = self.down_track_local.ssrc.lock().await.clone();

        let last_ssrc = self.down_track_local.last_ssrc.load(Ordering::Relaxed);
        let temporal_supported: bool;

        {
            let simulcast = &mut self.simulcast.lock().await;
            temporal_supported = simulcast.temporal_supported;
            if last_ssrc != ext_packet.packet.header.ssrc || re_sync {
                if re_sync && !ext_packet.key_frame {
                    let receiver = &self.down_track_local.receiver;
                    receiver.send_rtcp(vec![Box::new(PictureLossIndication {
                        sender_ssrc: ssrc,
                        media_ssrc: ext_packet.packet.header.ssrc,
                    })])?;
                    return Ok(());
                }

                if re_sync && simulcast.l_ts_calc != 0 {
                    simulcast.l_ts_calc = ext_packet.arrival;
                }

                if simulcast.temporal_supported {
                    let mime = self.down_track_local.mime.lock().await.clone();
                    if mime == String::from("video/vp8") {
                        let vp8 = ext_packet.payload;
                        simulcast.p_ref_pic_id = simulcast.l_pic_id;
                        simulcast.ref_pic_id = vp8.picture_id;
                        simulcast.p_ref_tlz_idx = simulcast.l_tlz_idx;
                        simulcast.ref_tlz_idx = vp8.tl0_picture_idx;
                    }
                }
                self.down_track_local
                    .re_sync
                    .store(false, Ordering::Relaxed);
            }

            if simulcast.l_ts_calc != 0 && last_ssrc != ext_packet.packet.header.ssrc {
                self.down_track_local
                    .last_ssrc
                    .store(ext_packet.packet.header.ssrc, Ordering::Relaxed);
                let tdiff = (ext_packet.arrival - simulcast.l_ts_calc) as f64 / 1e6;
                let mut td = (tdiff as u32 * 90) / 1000;
                if td == 0 {
                    td = 1;
                }
                let mut ts_offset = self.ts_offset.lock().await;
                *ts_offset = ext_packet.packet.header.timestamp - (*self.last_ts.lock().await + td);
                let mut sn_offset = self.sn_offset.lock().await;
                *sn_offset =
                    ext_packet.packet.header.sequence_number - *self.last_sn.lock().await - 1;
            } else if simulcast.l_ts_calc == 0 {
                let mut last_ts = self.last_ts.lock().await;
                *last_ts = ext_packet.packet.header.timestamp;
                let mut last_sn = self.last_sn.lock().await;
                *last_sn = ext_packet.packet.header.sequence_number;
                let mime = self.down_track_local.mime.lock().await.clone();
                if mime == String::from("video/vp8") {
                    let vp8 = ext_packet.payload;
                    simulcast.temporal_supported = vp8.temporal_supported;
                }
            }
        }

        let new_sn = ext_packet.packet.header.sequence_number - *self.sn_offset.lock().await;
        let new_ts = ext_packet.packet.header.timestamp - *self.ts_offset.lock().await;
        let payload = &ext_packet.packet.payload;

        let _pic_id: u16 = 0;
        let _tlz0_idx: u8 = 0;

        if temporal_supported {
            let mime = self.down_track_local.mime.lock().await.clone();
            if mime == String::from("video/vp8") {
                let (_a, _b, _c, _d) =
                    helpers::set_vp8_temporal_layer(ext_packet.clone(), self).await;
            }
        }

        self.octet_count
            .fetch_add(payload.len() as u32, Ordering::Relaxed);
        self.packet_count.fetch_add(1, Ordering::Relaxed);

        if ext_packet.head {
            *self.last_sn.lock().await = new_sn;
            *self.last_ts.lock().await = new_ts;
        }
        {
            let simulcast = &mut self.simulcast.lock().await;
            simulcast.l_ts_calc = ext_packet.arrival;
        }

        let mut cur_packet = ext_packet.clone();

        let hdr = &mut cur_packet.packet.header;
        hdr.sequence_number = new_sn;
        hdr.timestamp = new_ts;
        hdr.ssrc = ssrc;
        hdr.payload_type = self.down_track_local.payload_type.lock().await.clone();

        let write_stream_val = self.down_track_local.write_stream.lock().await;
        if let Some(write_stream) = &*write_stream_val {
            write_stream.write_rtp(&cur_packet.packet).await?;
        }

        Ok(())
    }

    #[allow(dead_code)]
    async fn handle_layer_change(
        self: &Arc<Self>,
        max_rate_packet_loss: u8,
        expected_min_bitrate: u64,
    ) {
        let current_spatial_layer = self.current_spatial_layer.load(Ordering::Relaxed);
        let target_spatial_layer = self.target_spatial_layer.load(Ordering::Relaxed);

        let temporal_layer = self.temporal_layer.load(Ordering::Relaxed);
        let current_temporal_layer = temporal_layer & 0x0f;
        let target_temporal_layer = temporal_layer >> 16;

        if target_spatial_layer == current_spatial_layer
            && current_temporal_layer == target_temporal_layer
        {
            let now = SystemTime::now();
            let simulcast = &mut self.simulcast.lock().await;
            if now > simulcast.switch_delay {
                let receiver = &self.down_track_local.receiver;
                let brs = receiver.get_bitrate().await;
                let cbr = brs[current_spatial_layer as usize];
                let mtl = receiver.get_max_temporal_layer().await;
                let mctl = mtl[current_spatial_layer as usize];

                if max_rate_packet_loss <= 5 {
                    if current_temporal_layer < mctl
                        && current_temporal_layer + 1
                            <= self.max_temporal_layer.load(Ordering::Relaxed)
                        && expected_min_bitrate >= 3 * cbr / 4
                    {
                        self.switch_temporal_layer(target_temporal_layer + 1, false)
                            .await;
                        simulcast.switch_delay = SystemTime::now() + Duration::from_secs(3);
                    }

                    if current_temporal_layer >= mctl
                        && expected_min_bitrate >= 3 * cbr / 2
                        && current_spatial_layer + 1
                            <= self.max_spatial_layer.load(Ordering::Relaxed)
                        && current_spatial_layer + 1 <= 2
                    {
                        match self
                            .switch_spatial_layer(current_spatial_layer + 1, false)
                            .await
                        {
                            Ok(_) => {
                                self.switch_temporal_layer(0, false).await;
                            }
                            Err(_) => {}
                        }

                        simulcast.switch_delay = SystemTime::now() + Duration::from_secs(5);
                    }
                }

                if max_rate_packet_loss >= 25 {
                    let simulcast = &mut self.simulcast.lock().await;
                    if (expected_min_bitrate <= 5 * cbr / 8 || current_temporal_layer == 0)
                        && current_spatial_layer > 0
                        && brs[current_spatial_layer as usize - 1] != 0
                    {
                        match self
                            .switch_spatial_layer(current_spatial_layer - 1, false)
                            .await
                        {
                            Err(_) => {
                                self.switch_temporal_layer(
                                    mtl[current_spatial_layer as usize - 1],
                                    false,
                                )
                                .await;
                            }
                            Ok(_) => {}
                        }
                        simulcast.switch_delay = SystemTime::now() + Duration::from_secs(10);
                    } else {
                        self.switch_temporal_layer(current_spatial_layer - 1, false)
                            .await;
                        simulcast.switch_delay = SystemTime::now() + Duration::from_secs(5);
                    }
                }
            }
        }
    }

    fn get_sr_status(&self) -> (u32, u32) {
        let octets = self.octet_count.load(Ordering::Relaxed);
        let packets = self.packet_count.load(Ordering::Relaxed);

        (octets, packets)
    }

    pub fn bound(&self) -> bool {
        self.down_track_local.bound.load(Ordering::Relaxed)
    }
    pub fn id(&self) -> String {
        self.down_track_local.id.clone()
    }
}
#[async_trait]
impl TrackLocal for DownTrack {
    async fn bind(&self, t: &TrackLocalContext) -> RTCResult<RTCRtpCodecParameters> {
        log::info!("TrackLocal bind.......");
        self.down_track_local.bind(t).await
    }

    async fn unbind(&self, t: &TrackLocalContext) -> RTCResult<()> {
        self.down_track_local.unbind(t).await
    }

    fn id(&self) -> &str {
        &self.down_track_local.id()
    }

    fn stream_id(&self) -> &str {
        &self.down_track_local.stream_id()
    }

    fn kind(&self) -> RTPCodecType {
        self.down_track_local.kind()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
