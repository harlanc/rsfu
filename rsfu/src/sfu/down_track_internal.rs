use super::helpers;
use super::receiver::Receiver;
use super::sequencer::AtomicSequencer;
use super::sequencer::PacketMeta;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::sync::Once;
use webrtc::error::Result;

use crate::buffer::factory::AtomicFactory;
use async_trait::async_trait;
use tokio::sync::Mutex;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::track::track_local::{TrackLocal, TrackLocalContext, TrackLocalWriter};

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

pub struct DownTrackInternal {
    pub id: String,
    pub bound: AtomicBool,
    pub mime: Mutex<String>,
    pub ssrc: Mutex<u32>,
    pub stream_id: String,
    max_track: i32,
    pub payload_type: Mutex<u8>,
    pub sequencer: Arc<Mutex<AtomicSequencer>>,
    buffer_factory: Mutex<AtomicFactory>,
    pub enabled: AtomicBool,
    pub re_sync: AtomicBool,
    pub last_ssrc: AtomicU32,
    pub codec: RTCRtpCodecCapability,
    pub receiver: Arc<dyn Receiver + Send + Sync>,
    pub write_stream: Mutex<Option<Arc<dyn TrackLocalWriter + Send + Sync>>>, //TrackLocalWriter,
    on_bind_handler: Arc<Mutex<Option<OnBindFn>>>,

    #[allow(dead_code)]
    close_once: Once,
    #[allow(dead_code)]
    octet_count: AtomicU32,
    #[allow(dead_code)]
    packet_count: AtomicU32,
    #[allow(dead_code)]
    max_packet_ts: u32,
}

impl PartialEq for DownTrackInternal {
    fn eq(&self, other: &Self) -> bool {
        (self.stream_id == other.stream_id) && (self.id == other.id)
    }

    // fn ne(&self, other: &Self) -> bool {
    //     true
    // }
}

impl DownTrackInternal {
    pub(super) async fn new(
        c: RTCRtpCodecCapability,
        r: Arc<dyn Receiver + Send + Sync>,
        mt: i32,
    ) -> Self {
        Self {
            codec: c,
            id: r.track_id(),
            bound: AtomicBool::new(false),
            mime: Mutex::new(String::from("")),
            ssrc: Mutex::new(0),
            stream_id: r.stream_id(),
            max_track: mt,
            payload_type: Mutex::new(0),
            sequencer: Arc::new(Mutex::new(AtomicSequencer::new(0))),
            buffer_factory: Mutex::new(AtomicFactory::new(1000, 1000)),
            enabled: AtomicBool::new(false),
            re_sync: AtomicBool::new(false),
            last_ssrc: AtomicU32::new(0),
            receiver: r.clone(),
            write_stream: Mutex::new(None),
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
        if !enabled {
            return;
        }

        let mut buf = &data[..];

        let pkts_result = rtcp::packet::unmarshal(&mut buf);
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
                    //todo
                }

             //   receiver.retransmit_packets(track, packets)

            }
        }

        if !fwd_pkts.is_empty() {
            if let Err(err) = receiver.send_rtcp(fwd_pkts) {
                log::error!("send_rtcp err:{}", err);
            }
        }

        // Ok(())
    }
}
#[async_trait]
impl TrackLocal for DownTrackInternal {
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
        let buffer_factory = self.buffer_factory.lock().await;

        let rtcp_buffer = buffer_factory.get_or_new_rtcp_buffer(t.ssrc()).await;
        let mut rtcp = rtcp_buffer.lock().await;

        let enabled = self.enabled.load(Ordering::Relaxed);
        let last_ssrc = self.last_ssrc.load(Ordering::Relaxed);

        let ssrc_val = *ssrc;

        let sequencer = self.sequencer.clone();
        let receiver = self.receiver.clone();

        rtcp.on_packet(Box::new(move |data: Vec<u8>| {
            let sequencer2 = sequencer.clone();
            let receiver2 = receiver.clone();
            Box::pin(async move {
                DownTrackInternal::handle_rtcp(
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

    async fn unbind(&self, _t: &TrackLocalContext) -> Result<()> {
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
}
