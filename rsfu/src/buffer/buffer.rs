use super::helpers;
use crate::buffer::errors::*;
use bytes::Buf;
use rtcp::packet::Packet as RtcpPacket;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::reception_report::ReceptionReport;
use rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
use rtp::extension::audio_level_extension::AudioLevelExtension;
use rtp::packet::Packet;
use sdp::extmap;
use std::time::Instant;
use webrtc::rtp_transceiver as webrtc_rtp;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc_util::{Marshal, MarshalSize, Unmarshal};

use super::errors::Result;
use async_trait::async_trait;
use std::borrow::BorrowMut;
use std::collections::VecDeque;
use std::future::Future;
use std::isize::MAX;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::time::{sleep, Duration};

use super::helpers::VP8;

use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use bytes::{BufMut, BytesMut};

use super::buffer_io::BufferIO;
use crate::buffer::bucket;
use crate::{buffer::bucket::Bucket, buffer::nack::NackQueue};

const MAX_SEQUENCE_NUMBER: u32 = 1 << 16;
const REPORT_DELTA: f64 = 1e9;

pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>) + Send + Sync>;
pub type OnTransportWideCCFn = Box<
    dyn (FnMut(u16, i64, bool) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync,
>;
pub type OnFeedbackCallBackFn = Box<
    dyn (FnMut(
            Vec<Box<dyn RtcpPacket + Send + Sync>>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnAudioLevelFn =
    Box<dyn (FnMut(u8) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub enum BufferPacketType {
    RTPBufferPacket = 1,
    RTCPBufferPacket = 2,
}

#[derive(Debug, Eq, PartialEq, Default, Clone)]
struct PendingPackets {
    arrival_time: u64,
    pub packet: Vec<u8>,
}
#[derive(Debug, Eq, PartialEq, Default, Clone)]
pub struct ExtPacket {
    pub head: bool,
    cycle: u32,
    pub arrival: i64,
    pub packet: Packet,
    pub key_frame: bool,
    pub payload: VP8,
}
#[derive(Debug, PartialEq, Default, Clone)]
pub struct Stats {
    pub last_expected: u32,
    pub last_received: u32,
    lost_rate: f32,
    pub packet_count: u32, // Number of packets received from this source.
    jitter: f64, // An estimate of the statistical variance of the RTP data packet inter-arrival time.
    pub total_byte: u64,
}
#[derive(Debug, Eq, PartialEq, Default, Clone)]
pub struct Options {
    pub max_bitrate: u64,
}
#[derive(Default, Clone)]
pub struct Buffer {
    bucket: Option<Bucket>,
    nacker: Option<NackQueue>,

    codec_type: RTPCodecType,
    ext_packets: VecDeque<ExtPacket>,
    pending_packets: Vec<PendingPackets>,
    media_ssrc: u32,
    clock_rate: u32,
    max_bitrate: u64,
    last_report: i64,
    twcc_ext: u8,
    audio_ext: u8,
    bound: bool,
    closed: bool,
    mime: String,

    // supported feedbacks
    remb: bool,
    nack: bool,
    twcc: bool,
    audio_level: bool,

    min_packet_probe: i32,
    last_packet_read: i32,

    pub max_temporal_layer: i32,
    pub bitrate: u64,
    bitrate_helper: u64,
    last_srntp_time: u64,
    last_srrtp_time: u32,
    last_sr_recv: i64, // Represents wall clock of the most recent sender report arrival
    base_sn: u16,
    cycles: u32,
    last_rtcp_packet_time: i64, // Time the last RTCP packet was received.
    last_rtcp_sr_time: i64, // Time the last RTCP SR was received. Required for DLSR computation.
    last_transit: u32,
    max_seq_no: u16, // The highest sequence number received in an RTP data packet

    stats: Stats,

    latest_timestamp: u32,      // latest received RTP timestamp on packet
    latest_timestamp_time: i64, // Time of the latest timestamp (in nanos since unix epoch)

    video_pool_len: usize,
    audio_pool_len: usize,
    //callbacks
    on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,
    on_transport_wide_cc_handler: Arc<Mutex<Option<OnTransportWideCCFn>>>,
    on_feedback_callback_handler: Arc<Mutex<Option<OnFeedbackCallBackFn>>>,
    on_audio_level_handler: Arc<Mutex<Option<OnAudioLevelFn>>>,
}

impl Buffer {
    pub fn new(ssrc: u32) -> Self {
        Self {
            media_ssrc: ssrc,
            video_pool_len: 1500 * 500,
            audio_pool_len: 1500 * 25,
            ..Default::default()
        }
    }
}

pub struct AtomicBuffer {
    buffer: Arc<Mutex<Buffer>>,
}

impl AtomicBuffer {
    pub fn new(ssrc: u32) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Buffer::new(ssrc))),
        }
    }

    pub async fn bind(&self, params: RTCRtpParameters, o: Options) {
        let mut buffer = self.buffer.lock().await;
        let codec = &params.codecs[0];
        buffer.clock_rate = codec.capability.clock_rate;
        buffer.max_bitrate = o.max_bitrate;
        buffer.mime = codec.capability.mime_type.to_lowercase();
        log::info!("Buffer bind: {}", buffer.mime);

        if buffer.mime.starts_with("audio/") {
            buffer.codec_type = RTPCodecType::Audio;
            buffer.bucket = Some(Bucket::new(buffer.audio_pool_len));
        } else if buffer.mime.starts_with("video/") {
            buffer.codec_type = RTPCodecType::Video;
            buffer.bucket = Some(Bucket::new(buffer.video_pool_len));
        } else {
            buffer.codec_type = RTPCodecType::Unspecified;
        }

        for ext in &params.header_extensions {
            if ext.uri == extmap::TRANSPORT_CC_URI {
                buffer.twcc_ext = ext.id as u8;
                break;
            }
        }

        match buffer.codec_type {
            RTPCodecType::Video => {
                for feedback in &codec.capability.rtcp_feedback {
                    match feedback.typ.clone().as_str() {
                        webrtc_rtp::TYPE_RTCP_FB_GOOG_REMB => {
                            buffer.remb = true;
                        }
                        webrtc_rtp::TYPE_RTCP_FB_TRANSPORT_CC => {
                            buffer.twcc = true;
                        }
                        webrtc_rtp::TYPE_RTCP_FB_NACK => {
                            buffer.nack = true;
                            buffer.nacker = Some(NackQueue::new());
                        }
                        _ => {}
                    }
                }
            }
            RTPCodecType::Audio => {
                for ext in &params.header_extensions {
                    if ext.uri == extmap::AUDIO_LEVEL_URI {
                        buffer.audio_level = true;
                        buffer.audio_ext = ext.id as u8;
                    }
                }
            }

            _ => {}
        }

        for pp in buffer.pending_packets.clone() {
            self.calc(&pp.packet[..], pp.arrival_time as i64).await;
        }
        buffer.pending_packets.clear();
        buffer.bound = true;
    }

    pub async fn read_extended(&self) -> Result<ExtPacket> {
        let codc_type = self.buffer.lock().await.codec_type; //= RTPCodecType::Video;
        loop {
            if codc_type == RTPCodecType::Video {
                //log::info!("read_extended 0");
            }
            if self.buffer.lock().await.closed {
                //log::info!("read_extended 0.1");
                return Err(Error::ErrIOEof.into());
            }
            if codc_type == RTPCodecType::Video {
                //log::info!("read_extended 1");
            }
            let ext_packets = &mut self.buffer.lock().await.ext_packets;
            if ext_packets.len() > 0 {
                if codc_type == RTPCodecType::Video {
                    //log::info!("read_extended 1.1");
                }
                let ext_pkt = ext_packets.pop_front().unwrap();
                if codc_type == RTPCodecType::Video {
                    //log::info!("read_extended 2");
                }
                return Ok(ext_pkt);
            }
            if codc_type == RTPCodecType::Video {
                //log::info!("read_extended 3");
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn on_close(&self, f: OnCloseFn) {
        let buffer = self.buffer.lock().await;
        let mut handler = buffer.on_close_handler.lock().await;
        *handler = Some(f);
    }

    



    pub async fn calc(&self, pkt: &[u8], arrival_time: i64) {
        // println!("calc 1....");
        let codc_type = self.buffer.lock().await.codec_type; //= RTPCodecType::Video;
        if codc_type == RTPCodecType::Video {
           // log::info!("calc 0");
        }
        let buffer = &mut self.buffer.lock().await;
        
        let sn = BigEndian::read_u16(&pkt[2..4]);  if codc_type == RTPCodecType::Video {
        log::info!("calc 2....: sn: {}",sn);}
        let distance = bucket::distance(sn, buffer.max_seq_no);  if codc_type == RTPCodecType::Video {
        log::info!("calc 3....: distance: {}",distance);}
        if buffer.stats.packet_count == 0 {
            buffer.base_sn = sn;
            buffer.max_seq_no = sn;
            buffer.last_report = arrival_time;
        } else if distance & 0x8000 == 0 {
            if sn < buffer.max_seq_no {
                buffer.cycles += MAX_SEQUENCE_NUMBER;
            }
            if buffer.nack {
                let diff = sn - buffer.max_seq_no;

                for i in 1..diff {
                    let ext_sn: u32;

                    let msn = sn - i;
                    if msn > buffer.max_seq_no
                        && (msn & 0x8000) > 0
                        && buffer.max_seq_no & 0x8000 == 0
                    {
                        ext_sn = (buffer.cycles - MAX_SEQUENCE_NUMBER) | msn as u32;
                    } else {
                        ext_sn = buffer.cycles | msn as u32;
                    }

                    if let Some(nacker) = buffer.nacker.as_mut() {
                        nacker.push(ext_sn);
                    }
                }
            }
            buffer.max_seq_no = sn;
        } else if buffer.nack && (distance & 0x8000 > 0) {
            let ext_sn: u32;

            if sn > buffer.max_seq_no && (sn & 0x8000) > 0 && buffer.max_seq_no & 0x8000 == 0 {
                ext_sn = (buffer.cycles - MAX_SEQUENCE_NUMBER) | sn as u32;
            } else {
                ext_sn = buffer.cycles | sn as u32;
            }
            if let Some(nacker) = buffer.nacker.as_mut() {
                nacker.remove(ext_sn);
            }
        }
        if codc_type == RTPCodecType::Video {
            //log::info!("calc 1");
        }
        let mut packet: Packet = Packet::default();

        let max_seq_no = buffer.max_seq_no;
        if let Some(bucket) = &mut buffer.bucket {
            let rv = bucket.add_packet(pkt, sn, sn == max_seq_no);

            match rv {
                Ok(data) => match Packet::unmarshal(&mut &data[..]) {
                    Err(_) => {
                        println!("calc error 0....");
                        return;
                    }
                    Ok(p) => {  if codc_type == RTPCodecType::Video {
                        log::info!("calc packet size: {}",data.len());}
                        packet = p;
                    }
                },
                Err(err) => {
                    //  if Error::ErrRTXPacket.equal(&rv) {
                    log::error!("add packet err: {}", err);
                    return;
                }
            }
        }
        if codc_type == RTPCodecType::Video {
            //log::info!("calc 2");
        }
        buffer.stats.total_byte += pkt.len() as u64;
        buffer.bitrate_helper += pkt.len() as u64;
        buffer.stats.packet_count += 1;

        let mut ep = ExtPacket {
            head: sn == buffer.max_seq_no,
            cycle: buffer.cycles,
            packet: packet.clone(),
            arrival: arrival_time,
            key_frame: false,
            ..Default::default()
        };

        match buffer.mime.as_str() {
            "video/vp8" => {
                let mut vp8_packet = helpers::VP8::default();
                if let Err(e) = vp8_packet.unmarshal(&packet.payload[..]) {
                    println!("calc error 3....: {}",e);
                    return;
                }
                ep.key_frame = vp8_packet.is_key_frame;
                ep.payload = vp8_packet;
            }
            "video/h264" => {
                ep.key_frame = helpers::is_h264_keyframe(&packet.payload[..]);
            }
            _ => {}
        }

        if buffer.min_packet_probe < 25 {
            if sn < buffer.base_sn {
                buffer.base_sn = sn
            }

            if buffer.mime == "video/vp8" {
                let pld = ep.payload;
                let mtl = buffer.max_temporal_layer;
                if mtl < pld.tid as i32 {
                    buffer.max_temporal_layer = pld.tid as i32;
                }
            }

            buffer.min_packet_probe += 1;
        }
        if codc_type == RTPCodecType::Video {
            //log::info!("calc 3");
        }
        buffer.ext_packets.push_back(ep);

        // if first time update or the timestamp is later (factoring timestamp wrap around)
        let latest_timestamp = buffer.latest_timestamp;
        let latest_timestamp_in_nanos_since_epoch = buffer.latest_timestamp_time;
        if (latest_timestamp_in_nanos_since_epoch == 0)
            || is_later_timestamp(packet.header.timestamp, latest_timestamp)
        {
            buffer.latest_timestamp = packet.header.timestamp;
            buffer.latest_timestamp_time = arrival_time;
        }

        let arrival = arrival_time as f64 / 1e6 * (buffer.clock_rate as f64 / 1e3);
        let transit = arrival - packet.header.timestamp as f64;
        if buffer.last_transit != 0 {
            let mut d = transit - buffer.last_transit as f64;
            if d < 0.0 {
                d *= -1.0;
            }

            buffer.stats.jitter += (d - buffer.stats.jitter) / 16 as f64;
        }
        buffer.last_transit = transit as u32;
        if codc_type == RTPCodecType::Video {
            //log::info!("calc 4");
        }
        if buffer.twcc {
            if let Some(mut ext) = packet.header.get_extension(buffer.twcc_ext) {
                if ext.len() > 1 {
                    let mut handler = buffer.on_transport_wide_cc_handler.lock().await;
                    if let Some(f) = &mut *handler {
                        f(ext.get_u16(), arrival_time, packet.header.marker).await;
                    }
                }
            }
        }

        if buffer.audio_level {
            if let Some(ext) = packet.header.get_extension(buffer.audio_ext) {
                let rv = AudioLevelExtension::unmarshal(&mut &ext[..]);

                if let Ok(data) = rv {
                    let mut handler = buffer.on_audio_level_handler.lock().await;
                    if let Some(f) = &mut *handler {
                        f(data.level).await;
                    }
                }
            }
        }
        if codc_type == RTPCodecType::Video {
           // log::info!("calc 5");
        }
        let diff = arrival_time - buffer.last_report;

        if buffer.nacker.is_some() {
            let rv = self.build_nack_packet(buffer).await;
            let mut handler = buffer.on_feedback_callback_handler.lock().await;
            if let Some(f) = &mut *handler {
                f(rv).await;
            }
        }
        if codc_type == RTPCodecType::Video {
            //log::info!("calc 6");
        }
        if diff as f64 >= REPORT_DELTA {
            let br = 8 * buffer.bitrate_helper * REPORT_DELTA as u64 / diff as u64;
            buffer.bitrate = br;
            buffer.last_report = arrival_time;
            buffer.bitrate_helper = 0;
        }
    }

    async fn build_nack_packet(
        &self,
        buffer: &mut Buffer,
    ) -> Vec<Box<dyn RtcpPacket + Send + Sync>> {
        let mut pkts: Vec<Box<dyn RtcpPacket + Send + Sync>> = Vec::new();

        if buffer.nacker == None {
            return pkts;
        }
        let seq_number = buffer.cycles | buffer.max_seq_no as u32;
        let (nacks, ask_key_frame) = buffer.nacker.as_mut().unwrap().pairs(seq_number);

        let mut nacks_len: usize = 0;
        if nacks.is_some() {
            nacks_len = nacks.as_ref().unwrap().len();
        }

        if nacks_len > 0 || ask_key_frame {
            if nacks_len > 0 {
                let pkt = TransportLayerNack {
                    media_ssrc: buffer.media_ssrc,
                    nacks: nacks.unwrap(),
                    ..Default::default()
                };
                pkts.push(Box::new(pkt));
            }

            if ask_key_frame {
                let pkt = PictureLossIndication {
                    media_ssrc: buffer.media_ssrc,
                    ..Default::default()
                };
                pkts.push(Box::new(pkt));
            }
        }

        pkts
    }

    async fn build_remb_packet(&self) -> ReceiverEstimatedMaximumBitrate {
        let mut buffer = self.buffer.lock().await;
        let mut br = buffer.bitrate;

        if buffer.stats.lost_rate < 0.02 {
            br = (br as f64 * 1.09) as u64 + 2000;
        }

        if buffer.stats.lost_rate > 0.1 {
            br = (br as f64 * (1.0 - 0.5 * buffer.stats.lost_rate) as f64) as u64;
        }

        if br > buffer.max_bitrate {
            br = buffer.max_bitrate;
        }

        if br < 100000 {
            br = 100000;
        }

        buffer.stats.total_byte = 0;

        return ReceiverEstimatedMaximumBitrate {
            bitrate: br as f32,
            ssrcs: vec![buffer.media_ssrc],
            ..Default::default()
        };
    }

    async fn build_reception_report(&self) -> ReceptionReport {
        let mut buffer = self.buffer.lock().await;

        let ext_max_seq = buffer.cycles | buffer.max_seq_no as u32;
        let expected = ext_max_seq - buffer.base_sn as u32 + 1;
        let mut lost: u32 = 0;

        if buffer.stats.packet_count < expected && buffer.stats.packet_count != 0 {
            lost = expected - buffer.stats.packet_count;
        }

        let expected_interval = expected - buffer.stats.last_expected;
        buffer.stats.last_expected = expected;

        let received_interval = buffer.stats.packet_count - buffer.stats.last_received;
        buffer.stats.last_received = buffer.stats.packet_count;

        let lost_interval = expected_interval - received_interval;

        buffer.stats.lost_rate = lost_interval as f32 / expected_interval as f32;

        let mut frac_lost: u8 = 0;
        if expected_interval != 0 && lost_interval > 0 {
            frac_lost = ((lost_interval << 8) / expected_interval) as u8;
        }

        let mut dlsr: u32 = 0;

        if buffer.last_sr_recv != 0 {
            let delay_ms = ((Instant::now().elapsed().subsec_nanos() - buffer.last_sr_recv as u32)
                as f64
                / 1e6) as u32;

            dlsr = ((delay_ms as f32 / 1e3) as u32) << 16;

            dlsr |= (delay_ms as f32 % 1e3) as u32 * 65536 / 1000;
        }

        ReceptionReport {
            ssrc: buffer.media_ssrc,
            fraction_lost: frac_lost,
            total_lost: lost,
            last_sequence_number: ext_max_seq,
            jitter: buffer.stats.jitter as u32,
            last_sender_report: buffer.last_srrtp_time >> 16,
            delay: dlsr,
        }
    }

    pub async fn set_sender_report_data(&self, rtp_time: u32, ntp_time: u64) {
        let mut buffer = self.buffer.lock().await;

        buffer.last_srrtp_time = rtp_time;
        buffer.last_srntp_time = ntp_time;
        buffer.last_sr_recv = Instant::now().elapsed().subsec_nanos() as i64;
    }

    async fn get_rtcp(&mut self) -> Vec<Box<dyn RtcpPacket>> {
        let mut buffer = self.buffer.lock().await;

        let mut rtcp_packets: Vec<Box<dyn RtcpPacket>> = Vec::new();
        rtcp_packets.push(Box::new(self.build_reception_report().await));

        if buffer.remb && !buffer.twcc {
            rtcp_packets.push(Box::new(self.build_remb_packet().await));
        }

        rtcp_packets
    }

    pub async fn get_packet(&self, buff: &mut [u8], sn: u16) -> Result<usize> {
        let buffer = self.buffer.lock().await;

        if buffer.closed {
            return Err(Error::ErrIOEof.into());
        }

        if let Some(bucket) = &buffer.bucket {
            return bucket.get_packet(buff, sn);
        }

        Ok(0)
    }

    pub async fn bitrate(&self) -> u64 {
        self.buffer.lock().await.bitrate
    }

    pub async fn max_temporal_layer(&self) -> i32 {
        self.buffer.lock().await.max_temporal_layer
    }

    pub async fn on_transport_wide_cc(&self, f: OnTransportWideCCFn) {
        let buffer = self.buffer.lock().await;
        let mut handler = buffer.on_transport_wide_cc_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_feedback_callback(&self, f: OnFeedbackCallBackFn) {
        let buffer = self.buffer.lock().await;
        let mut handler = buffer.on_feedback_callback_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_audio_level(&self, f: OnAudioLevelFn) {
        let buffer = self.buffer.lock().await;
        let mut handler = buffer.on_audio_level_handler.lock().await;
        *handler = Some(f);
    }

    async fn get_media_ssrc(&self) -> u32 {
        self.buffer.lock().await.media_ssrc
    }

    async fn get_clock_rate(&self) -> u32 {
        self.buffer.lock().await.clock_rate
    }

    pub async fn get_sender_report_data(&self) -> (u32, u64, i64) {
        let buffer = self.buffer.lock().await;
        (
            buffer.last_srrtp_time,
            buffer.last_srntp_time,
            buffer.last_sr_recv,
        )
    }

    pub async fn get_status(&self) -> Stats {
        self.buffer.lock().await.stats.clone()
    }

    async fn get_latest_timestamp(&self) -> (u32, i64) {
        let buffer = self.buffer.lock().await;
        (buffer.latest_timestamp, buffer.latest_timestamp_time)
    }
}

#[async_trait]
impl BufferIO for AtomicBuffer {
    // Write adds a RTP Packet, out of order, new packet may be arrived later
    async fn write(&self, pkt: &[u8]) -> Result<u32> {
        //println!("buffer write begin....");
        {
            let mut buffer = self.buffer.lock().await;
            //println!("buffer write begin 1....");
            if !buffer.bound {
                buffer.pending_packets.push(PendingPackets {
                    arrival_time: Instant::now().elapsed().subsec_nanos() as u64,
                    packet: Vec::from(pkt),
                });

                return Ok(0);
            }
        }

        //println!("buffer write begin 2....");
        self.calc(pkt, Instant::now().elapsed().subsec_nanos() as i64)
            .await;

        Ok(0)
    }

    async fn read(&mut self, buff: &mut [u8]) -> Result<usize> {
        let buffer = self.buffer.lock().await;
        if buffer.closed {
            return Err(Error::ErrIOEof.into());
        }

        let mut n: usize = 0;

        if buffer.pending_packets.len() > buffer.last_packet_read as usize {
            if buff.len()
                < buffer
                    .pending_packets
                    .get(buffer.last_packet_read as usize)
                    .unwrap()
                    .packet
                    .len()
            {
                return Err(Error::ErrBufferTooSmall.into());
            }

            let packet = &buffer
                .pending_packets
                .get(buffer.last_packet_read as usize)
                .unwrap()
                .packet;

            n = packet.len();

            buff.copy_from_slice(&packet[..]);
            return Ok(n);
        }

        Ok(n)
    }
    async fn close(&self) -> Result<()> {
        let buffer = self.buffer.lock().await;
        if buffer.bucket.is_some() && buffer.codec_type == RTPCodecType::Video {}

        Ok(())
    }
}

// is_timestamp_wrap_around returns true if wrap around happens from timestamp1 to timestamp2
pub fn is_timestamp_wrap_around(timestamp1: u32, timestamp2: u32) -> bool {
    return (timestamp1 & 0xC000000 == 0) && (timestamp2 & 0xC000000 == 0xC000000);
}

// is_later_timestamp returns true if timestamp1 is later in time than timestamp2 factoring in timestamp wrap-around
fn is_later_timestamp(timestamp1: u32, timestamp2: u32) -> bool {
    if timestamp1 > timestamp2 {
        if is_timestamp_wrap_around(timestamp2, timestamp1) {
            return false;
        }
        return true;
    }
    if is_timestamp_wrap_around(timestamp1, timestamp2) {
        return true;
    }
    return false;
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_u16() {
        let number1: u16 = 0;
        let number2: u16 = 2;

        // println!("{}", number1 - number2);
    }
}
