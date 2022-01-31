// DownTrackType determines the type of track
//type DownTrackType =  u16;

use super::sequencer::{self, AtomicSequencer};
use super::simulcast::SimulcastTrackHelpers;
use anyhow::Result;
use atomic::Atomic;
use rtp::extension::audio_level_extension::AudioLevelExtension;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};

use super::helpers;
use super::receiver::Receiver;
use super::sequencer::PacketMeta;
use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::Mutex;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::RTCRtpTransceiver;

use crate::buffer::factory::AtomicFactory;

use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
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

    current_spatial_layer: i32,
    target_spatial_layer: i32,
    pub temporal_layer: AtomicI32,

    enabled: AtomicBool,
    re_sync: AtomicBool,
    sn_offset: u16,
    ts_offset: u32,
    last_ssrc: AtomicU32,
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
            sequencer: Arc::new(AtomicSequencer::new(0)),
            track_type: DownTrackType::SimpleDownTrack,
            buffer_factory: AtomicFactory::new(1000, 1000),
            payload: Vec::new(),

            current_spatial_layer: 0,
            target_spatial_layer: 0,
            temporal_layer: AtomicI32::new(0),

            enabled: AtomicBool::new(false),
            re_sync: AtomicBool::new(false),
            sn_offset: 0,
            ts_offset: 0,
            last_ssrc: AtomicU32::new(0),
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

        //Box::new(move |d: Arc<RTCDataChannel>| {

            let enabled = self.enabled.load(Ordering::Relaxed);
            let last_ssrc = self.last_ssrc.load(Ordering::Relaxed);
            let ssrc = self.ssrc;
            let sequencer = self.sequencer.clone();
            let receiver = self.receiver.clone();
        rtcp.on_packet(Box::new(move |data:&[u8]| {
            Box::pin(async move {
                DownTrack::handle_rtcp(enabled,data,last_ssrc,ssrc,sequencer,receiver).await
            })
        }));

        Ok(codec)
    }

    async fn handle_rtcp(enabled: bool,data: &[u8],last_ssrc:u32,ssrc:u32,sequencer:Arc<AtomicSequencer>,receiver:  Arc<dyn Receiver + Send + Sync>) -> Result<()> {
        // let enabled = self.enabled.load(Ordering::Relaxed);
        if !enabled {
            return Ok(());
        }
    
        let mut buf = data;
    
        let mut pkts = rtcp::packet::unmarshal(&mut buf)?;
    
        let mut fwd_pkts: Vec<Box<dyn RtcpPacket>> = Vec::new();
        let mut pli_once = true;
        let mut fir_once = true;
    
        let mut max_rate_packet_loss: u8 = 0;
        let mut expected_min_bitrate: u64 = 0;
    
    
        if last_ssrc == 0 {
            return Ok(());
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

                receiver.retransmit_packets(track, packets)
    
            }
        }
    
        if fwd_pkts.len() > 0{

            receiver.send_rtcp(fwd_pkts);

        }
    
        Ok(())
    }

    
}


