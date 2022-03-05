// DownTrackType determines the type of track
//type DownTrackType =  u16;

use super::sequencer::{self, AtomicSequencer};
use super::simulcast::SimulcastTrackHelpers;
use atomic::Atomic;
use rtp::extension::audio_level_extension::AudioLevelExtension;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use webrtc::error::{Error as WEBRTCError, Result};

use super::helpers;
use super::receiver::Receiver;
use super::sequencer::PacketMeta;
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

use crate::buffer::factory::AtomicFactory;

use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::{TrackLocal, TrackLocalContext, TrackLocalWriter};

use rtcp::packet::unmarshal;
use rtcp::packet::Packet as RtcpPacket;

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
    sequencer: Arc<AtomicSequencer>,
    track_type: DownTrackType,
    buffer_factory: AtomicFactory,
    pub payload: Vec<u8>,

    current_spatial_layer: AtomicI32,
    target_spatial_layer: AtomicI32,
    pub temporal_layer: AtomicI32,

    enabled: AtomicBool,
    re_sync: AtomicBool,
    sn_offset: u16,
    ts_offset: u32,
    last_ssrc: AtomicU32,
    last_sn: u16,
    last_ts: u32,

    pub simulcast: SimulcastTrackHelpers,
    max_spatial_layer: AtomicI32,
    max_temporal_layer: AtomicI32,

    codec: RTCRtpCodecCapability,
    receiver: Arc<dyn Receiver + Send + Sync>,
    transceiver: Option<RTCRtpTransceiver>,
    write_stream: Option<Arc<dyn TrackLocalWriter + Send + Sync>>, //TrackLocalWriter,
    on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,
    on_bind_handler: Arc<Mutex<Option<OnBindFn>>>,

    close_once: Once,

    octet_count: AtomicU32,
    packet_count: AtomicU32,
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
            sequencer: Arc::new(AtomicSequencer::new(0)),
            track_type: DownTrackType::SimpleDownTrack,
            buffer_factory: AtomicFactory::new(1000, 1000),
            payload: Vec::new(),

            current_spatial_layer: AtomicI32::new(0),
            target_spatial_layer: AtomicI32::new(0),
            temporal_layer: AtomicI32::new(0),

            enabled: AtomicBool::new(false),
            re_sync: AtomicBool::new(false),
            sn_offset: 0,
            ts_offset: 0,
            last_ssrc: AtomicU32::new(0),
            last_sn: 0,
            last_ts: 0,

            simulcast: SimulcastTrackHelpers::new(),
            max_spatial_layer: AtomicI32::new(0),
            max_temporal_layer: AtomicI32::new(0),

            codec: c,
            receiver: r,
            transceiver: None,
            write_stream: None,
            on_close_handler: Arc::default(),
            on_bind_handler: Arc::default(),

            close_once: Once::new(),

            octet_count: AtomicU32::new(0),
            packet_count: AtomicU32::new(0),
            max_packet_ts: 0,
        }
    }

    async fn bind(&mut self, t: TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        let parameters = RTCRtpCodecParameters {
            capability: self.codec.clone(),
            ..Default::default()
        };

        let codec = helpers::codec_parameters_fuzzy_search(parameters, t.codec_parameters())?;

        self.ssrc = t.ssrc() as u32;
        self.payload_type = codec.payload_type;
        self.write_stream = t.write_stream();
        self.mime = codec.capability.mime_type.to_lowercase();
        self.re_sync.store(true, Ordering::Relaxed);
        self.enabled.store(true, Ordering::Relaxed);

        let rtcp_buffer = self.buffer_factory.get_or_new_rtcp_buffer(t.ssrc()).await;
        let mut rtcp = rtcp_buffer.lock().await;

        let enabled = self.enabled.load(Ordering::Relaxed);
        let last_ssrc = self.last_ssrc.load(Ordering::Relaxed);
        let ssrc = self.ssrc;
        let sequencer = self.sequencer.clone();
        let receiver = self.receiver.clone();

        rtcp.on_packet(Box::new(move |data: Vec<u8>| {
            let sequencer2 = sequencer.clone();
            let receiver2 = receiver.clone();
            Box::pin(async move {
                DownTrack::handle_rtcp(enabled, data, last_ssrc, ssrc, sequencer2, receiver2).await
            })
        }))
        .await;

        if self.codec.mime_type.starts_with("video/") {
            self.sequencer = Arc::new(AtomicSequencer::new(self.max_track));
        }

        let mut handler = self.on_bind_handler.lock().await;
        if let Some(f) = &mut *handler {
            f().await;
        }
        self.bound.store(true, Ordering::Relaxed);
        Ok(codec)
    }

    fn unbind(&mut self, t: TrackLocalContext) {
        self.bound.store(false, Ordering::Relaxed);
    }

    fn id(&self) -> String {
        self.id.clone()
    }

    fn codec(&self) -> RTCRtpCodecCapability {
        self.codec.clone()
    }

    fn stream_id(&self) -> String {
        self.stream_id.clone()
    }

    fn kind(&self) -> RTPCodecType {
        if self.codec.mime_type.starts_with("audio/") {
            return RTPCodecType::Audio;
        }

        if self.codec.mime_type.starts_with("video/") {
            return RTPCodecType::Video;
        }

        RTPCodecType::Unspecified
    }

    async fn stop(&self) -> Result<()> {
        if let Some(transceiver) = &self.transceiver {
            return transceiver.stop().await;
        }

        Err(WEBRTCError::new(String::from("transceiver not exists")))
    }

    fn set_transceiver(&mut self, transceiver: RTCRtpTransceiver) {
        self.transceiver = Some(transceiver)
    }

    fn write_rtp(&mut self) -> Result<()> {
        if !self.enabled.load(Ordering::Relaxed) || !self.bound.load(Ordering::Relaxed) {
            return Ok(());
        }

        match self.track_type {
            DownTrackType::SimpleDownTrack => {}
            DownTrackType::SimulcastDownTrack => {}
        }

        Ok(())
    }

    fn enabled(&mut self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn mute(&mut self, val: bool) {
        if self.enabled() != val {
            return;
        }
        self.enabled.store(!val, Ordering::Relaxed);
        if val {
            self.re_sync.store(val, Ordering::Relaxed);
        }
    }

    async fn close(&mut self) {
        // self.close_once.call_once(|| {
        let mut handler = self.on_close_handler.lock().await;
        if let Some(f) = &mut *handler {
            f().await;
        }
        // });
    }

    fn set_initial_layers(&mut self, spatial_layer: i32, temporal_layer: i32) {
        self.current_spatial_layer
            .store(spatial_layer, Ordering::Relaxed);
        self.target_spatial_layer
            .store(spatial_layer, Ordering::Relaxed);
        self.temporal_layer
            .store(temporal_layer << 16 | temporal_layer, Ordering::Relaxed);
    }

    fn current_spatial_layer(&self) -> i32 {
        self.current_spatial_layer.load(Ordering::Relaxed)
    }

    fn switch_spatial_layer(self: &Arc<Self>, target_layer: i32, set_as_max: bool) -> Result<()> {
        match self.track_type {
            DownTrackType::SimulcastDownTrack => {
                let csl = self.current_spatial_layer.load(Ordering::Relaxed);
                if csl != self.target_spatial_layer.load(Ordering::Relaxed) || csl == target_layer {
                    return Err(WEBRTCError::new(String::from("error spatial layer busy..")));
                }
                match self
                    .receiver
                    .switch_down_track(Arc::downgrade(self), target_layer as u8)
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

        Err(WEBRTCError::new(String::from(
            "Error spatial not supported.",
        )))
    }

    fn switch_spatial_layer_done(&mut self, layer: i32) {
        self.current_spatial_layer.store(layer, Ordering::Relaxed);
    }

    fn untrack_layers_change(self: &Arc<Self>, available_layers: &[u16]) -> Result<i64> {
        match self.track_type {
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

                let mut target_layer: u16 = 0;
                if layer_found {
                    target_layer = max_found;
                } else {
                    target_layer = min_found;
                }

                if current_layer != target_layer {
                    if let Err(_) = self.switch_spatial_layer(target_layer as i32, false) {
                        return Ok(target_layer as i64);
                    }
                }

                return Ok(target_layer as i64);
            }
            _ => {}
        }
        Err(WEBRTCError::new(format!(
            "downtrack {} does not support simulcast",
            self.id
        )))
    }

    fn switch_temporal_layer(&mut self, target_layer: i32, set_as_max: bool) {
        match self.track_type {
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

    async fn on_close_hander(&mut self, f: OnCloseFn) {
        let mut handler = self.on_close_handler.lock().await;
        *handler = Some(f);
    }

    async fn on_bind(&mut self, f: OnBindFn) {
        let mut handler = self.on_bind_handler.lock().await;
        *handler = Some(f);
    }

    async fn create_source_description_chunks(&self) -> Option<Vec<SourceDescriptionChunk>> {
        if !self.bound.load(Ordering::Relaxed) {
            return None;
        }

        let mid = self.transceiver.as_ref().unwrap().mid().await;

        Some(vec![
            SourceDescriptionChunk {
                source: self.ssrc,
                items: vec![SourceDescriptionItem {
                    sdes_type: SdesType::SdesCname,
                    text: Bytes::copy_from_slice(self.stream_id.as_bytes()),
                }],
            },
            SourceDescriptionChunk {
                source: self.ssrc,
                items: vec![SourceDescriptionItem {
                    sdes_type: SdesType::SdesCname,
                    text: Bytes::copy_from_slice(mid.as_bytes()),
                }],
            },
        ])
    }

    fn create_sender_report(&self) -> Option<SenderReport> {
        if !self.bound.load(Ordering::Relaxed) {
            return None;
        }

        let (sr_rtp, sr_ntp) = self
            .receiver
            .get_sender_report_time(self.current_spatial_layer.load(Ordering::Relaxed) as u8);

        if sr_rtp == 0 {
            return None;
        }

        let now = SystemTime::now();
        let now_ntp = helpers::to_ntp_time(now);

        let clock_rate = self.codec.clock_rate;

        //todo
        let mut diff: u32 = 0;
        if diff < 0 {
            diff = 0;
        }

        let (octets, packets) = self.get_sr_status();

        Some(SenderReport {
            ssrc: self.ssrc,
            ntp_time: u64::from(now_ntp),
            rtp_time: sr_rtp + diff,
            packet_count: packets,
            octet_count: octets,
            ..Default::default()
        })
    }

    fn get_sr_status(&self) -> (u32, u32) {
        let octets = self.octet_count.load(Ordering::Relaxed);
        let packets = self.packet_count.load(Ordering::Relaxed);

        (octets, packets)
    }

    async fn handle_rtcp(
        enabled: bool,
        data: Vec<u8>,
        last_ssrc: u32,
        ssrc: u32,
        sequencer: Arc<AtomicSequencer>,
        receiver: Arc<dyn Receiver + Send + Sync>,
    ) {
        // let enabled = self.enabled.load(Ordering::Relaxed);
        if !enabled {
            return;
        }

        let mut buf = &data[..];

        let mut pkts_result = rtcp::packet::unmarshal(&mut buf);
        let mut pkts;

        match pkts_result {
            Ok(pkts_rv) => {
                pkts = pkts_rv;
            }
            Err(_) => {
                return;
            }
        }

        let mut fwd_pkts: Vec<Box<dyn RtcpPacket + Send + Sync>> = Vec::new();
        let mut pli_once = true;
        let mut fir_once = true;

        let mut max_rate_packet_loss: u8 = 0;
        let mut expected_min_bitrate: u64 = 0;

        if last_ssrc == 0 {
            return;
        }

        for pkt in &mut pkts {
            if let Some(pic_loss_indication) = pkt
                    .as_any()
                    .downcast_ref::<rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication>()
                {
                    if pli_once {
                        let mut pli = pic_loss_indication.clone();
                        pli.media_ssrc = last_ssrc;
                        pli.sender_ssrc = ssrc;

                        fwd_pkts.push(Box::new(pli));
                        pli_once = false;
                    }
                }
            else if let Some(full_intra_request) = pkt
                    .as_any()
                    .downcast_ref::<rtcp::payload_feedbacks::full_intra_request::FullIntraRequest>()
            {
                if fir_once{
                    let mut fir = full_intra_request.clone();
                    fir.media_ssrc = last_ssrc;
                    fir.sender_ssrc = ssrc;

                    fwd_pkts.push(Box::new(fir));
                    fir_once = false;
                }
            }
            else if let Some(receiver_estimated_max_bitrate) = pkt
                    .as_any()
                    .downcast_ref::<rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate>()
                    {
                if expected_min_bitrate == 0 || expected_min_bitrate > receiver_estimated_max_bitrate.bitrate as u64{
                    expected_min_bitrate = receiver_estimated_max_bitrate.bitrate as u64;

                }
            }
            else if let Some(receiver_report) = pkt.as_any().downcast_ref::<rtcp::receiver_report::ReceiverReport>(){

                for r in &receiver_report.reports{
                    if max_rate_packet_loss == 0 || max_rate_packet_loss < r.fraction_lost{
                        max_rate_packet_loss = r.fraction_lost;
                    }

                }

            }
            else if let Some(transport_layer_nack) = pkt.as_any().downcast_ref::<rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack>()
            {
                let mut nacked_packets:Vec<PacketMeta> = Vec::new();
                for pair in &transport_layer_nack.nacks{

                                 let seq_numbers = pair.packet_list();
                    // let sequencer = sequencer.lock.await;

                    let mut pairs= sequencer.get_seq_no_pairs(&seq_numbers[..]).await;

                    nacked_packets.append(&mut pairs);

                   // self.receiver.r

                }

             //   receiver.retransmit_packets(track, packets)

            }
        }

        if fwd_pkts.len() > 0 {
            receiver.send_rtcp(fwd_pkts);
        }

        // Ok(())
    }
}
