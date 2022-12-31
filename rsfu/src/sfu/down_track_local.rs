// DownTrackType determines the type of track
//type DownTrackType =  u16;

use super::sequencer::{self, AtomicSequencer};
use super::simulcast::SimulcastTrackHelpers;
use atomic::Atomic;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
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
use tokio::sync::{Mutex, MutexGuard};
use webrtc::rtcp::sender_report::SenderReport;
use webrtc::rtcp::source_description::SdesType;
use webrtc::rtcp::source_description::SourceDescriptionChunk;
use webrtc::rtcp::source_description::SourceDescriptionItem;
use webrtc::rtp::packet::Packet as RTPPacket;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::RTCRtpTransceiver;

use async_trait::async_trait;
use tokio::time::{sleep, Duration};

use crate::buffer::buffer::ExtPacket;
use crate::buffer::factory::AtomicFactory;

use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::{TrackLocal, TrackLocalContext, TrackLocalWriter};

use rtcp::packet::unmarshal;
use rtcp::packet::Packet as RtcpPacket;
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

pub struct DownTrackLocal {
    pub id: String,
    // peer_id: String,
    pub bound: AtomicBool,
    pub mime: Mutex<String>,
    pub ssrc: Mutex<u32>,
    pub stream_id: String,
    max_track: i32,
    pub payload_type: Mutex<u8>,
    pub sequencer: Arc<Mutex<AtomicSequencer>>,
    // track_type: Mutex<DownTrackType>,
    buffer_factory: Mutex<AtomicFactory>,
    // pub payload: Vec<u8>,

    // current_spatial_layer: AtomicI32,
    // target_spatial_layer: AtomicI32,
    // pub temporal_layer: AtomicI32,
    pub enabled: AtomicBool,
    pub re_sync: AtomicBool,
    // sn_offset: Mutex<u16>,
    // ts_offset: Mutex<u32>,
    pub last_ssrc: AtomicU32,
    // last_sn: Mutex<u16>,
    // last_ts: Mutex<u32>,

    // pub simulcast: Arc<Mutex<SimulcastTrackHelpers>>,
    // max_spatial_layer: AtomicI32,
    // max_temporal_layer: AtomicI32,
    pub codec: RTCRtpCodecCapability,
    pub receiver: Arc<dyn Receiver + Send + Sync>,
    // pub transceiver: Option<Arc<RTCRtpTransceiver>>,
    pub write_stream: Mutex<Option<Arc<dyn TrackLocalWriter + Send + Sync>>>, //TrackLocalWriter,
    // pub on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,
    on_bind_handler: Arc<Mutex<Option<OnBindFn>>>,
    close_once: Once,

    octet_count: AtomicU32,
    packet_count: AtomicU32,
    max_packet_ts: u32,
}

impl PartialEq for DownTrackLocal {
    fn eq(&self, other: &Self) -> bool {
        return (self.stream_id == other.stream_id) && (self.id == other.id);
    }

    // fn ne(&self, other: &Self) -> bool {
    //     true
    // }
}

impl DownTrackLocal {
    pub(super) async fn new(
        c: RTCRtpCodecCapability,
        r: Arc<dyn Receiver + Send + Sync>,
        mt: i32,
    ) -> Self {
        //let receiver = r.lock().await;
        Self {
            codec: c,
            id: r.track_id(),
            // peer_id: peer_id,
            bound: AtomicBool::new(false),
            mime: Mutex::new(String::from("")),
            ssrc: Mutex::new(0),
            stream_id: r.stream_id(),
            max_track: mt,
            payload_type: Mutex::new(0),
            sequencer: Arc::new(Mutex::new(AtomicSequencer::new(0))),
            // track_type: Mutex::new(DownTrackType::SimpleDownTrack),
            buffer_factory: Mutex::new(AtomicFactory::new(1000, 1000)),
            // payload: Vec::new(),

            // current_spatial_layer: AtomicI32::new(0),
            // target_spatial_layer: AtomicI32::new(0),
            // temporal_layer: AtomicI32::new(0),
            enabled: AtomicBool::new(false),
            re_sync: AtomicBool::new(false),
            // sn_offset: Mutex::new(0),
            // ts_offset: Mutex::new(0),
            last_ssrc: AtomicU32::new(0),
            // last_sn: Mutex::new(0),
            // last_ts: Mutex::new(0),

            // simulcast: Arc::new(Mutex::new(SimulcastTrackHelpers::new())),
            // max_spatial_layer: AtomicI32::new(0),
            // max_temporal_layer: AtomicI32::new(0),
            receiver: r.clone(),
            // transceiver: None,
            write_stream: Mutex::new(None),
            // on_close_handler: Arc::default(),
            on_bind_handler: Arc::default(),
            close_once: Once::new(),

            octet_count: AtomicU32::new(0),
            packet_count: AtomicU32::new(0),
            max_packet_ts: 0,
        }
    }

    pub async fn on_bind(&self, f: OnBindFn) {
        let mut handler = self.on_bind_handler.lock().await;
        *handler = Some(f);
    }

    async fn handle_rtcp(
        enabled: bool,
        data: Vec<u8>,
        last_ssrc: u32,
        ssrc: u32,
        sequencer: Arc<Mutex<AtomicSequencer>>,
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
                     let sequencer2 = sequencer.lock().await;

                    let mut pairs= sequencer2.get_seq_no_pairs(&seq_numbers[..]).await;

                    nacked_packets.append(&mut pairs);

                   // self.receiver.r

                   //todo

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
#[async_trait]
impl TrackLocal for DownTrackLocal {
    // async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters>;

    async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        let parameters = RTCRtpCodecParameters {
            capability: self.codec.clone(),
            ..Default::default()
        };

        let codec = helpers::codec_parameters_fuzzy_search(parameters, t.codec_parameters())?;

        let mut ssrc = self.ssrc.lock().await;
        *ssrc = t.ssrc() as u32;
        let mut payload_type = self.payload_type.lock().await;
        *payload_type = codec.payload_type;
        let mut write_stream = self.write_stream.lock().await;
        *write_stream = t.write_stream();
        let mut mime = self.mime.lock().await;
        *mime = codec.capability.mime_type.to_lowercase();
        self.re_sync.store(true, Ordering::Relaxed);
        self.enabled.store(true, Ordering::Relaxed);

        let mut buffer_factory = self.buffer_factory.lock().await;

        let rtcp_buffer = buffer_factory.get_or_new_rtcp_buffer(t.ssrc()).await;
        let mut rtcp = rtcp_buffer.lock().await;

        let enabled = self.enabled.load(Ordering::Relaxed);
        let last_ssrc = self.last_ssrc.load(Ordering::Relaxed);

        let ssrc_val = ssrc.clone();

        let sequencer = self.sequencer.clone();
        let receiver = self.receiver.clone();

        rtcp.on_packet(Box::new(move |data: Vec<u8>| {
            let sequencer2 = sequencer.clone();
            let receiver2 = receiver.clone();
            Box::pin(async move {
                DownTrackLocal::handle_rtcp(
                    enabled, data, last_ssrc, ssrc_val, sequencer2, receiver2,
                )
                .await;
                Ok(())
            })
        }))
        .await;

        if self.codec.mime_type.starts_with("video/") {
            let mut sequencer = self.sequencer.lock().await;
            *sequencer = AtomicSequencer::new(self.max_track);
        }

        let mut handler = self.on_bind_handler.lock().await;
        if let Some(f) = &mut *handler {
            f().await;
        }
        self.bound.store(true, Ordering::Relaxed);
        Ok(codec)
    }

    // /// unbind should implement the teardown logic when the track is no longer needed. This happens
    // /// because a track has been stopped.
    // async fn unbind(&self, t: &TrackLocalContext) -> Result<()>;

    async fn unbind(&self, t: &TrackLocalContext) -> Result<()> {
        self.bound.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn stream_id(&self) -> &str {
        &self.stream_id
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    // /// id is the unique identifier for this Track. This should be unique for the
    // /// stream, but doesn't have to globally unique. A common example would be 'audio' or 'video'
    // /// and stream_id would be 'desktop' or 'webcam'
    // fn id(&self) -> &str;

    // /// stream_id is the group this track belongs too. This must be unique
    // fn stream_id(&self) -> &str;

    // /// kind controls if this TrackLocal is audio or video
    // fn kind(&self) -> RTPCodecType;

    // fn as_any(&self) -> &dyn Any;
}
