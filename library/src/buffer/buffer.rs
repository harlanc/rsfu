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
use crate::{buffer::bucket::Bucket, buffer::nack::NackQueue};

const MAX_SEQUENCE_NUMBER: u32 = 1 << 16;
const REPORT_DELTA: f64 = 1e9;

pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>) + Send + Sync>;
pub type OnTransportWideCCFn = Box<
    dyn (FnMut(u16, i64, bool) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;
pub type OnFeedbackCallBackFn = Box<
    dyn (FnMut(
            Vec<Box<dyn RtcpPacket>>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>)
        + Send
        + Sync,
>;
pub type OnAudioLevelFn = Box<
    dyn (FnMut(u8) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>) + Send + Sync,
>;

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
    max_bitrate: u64,
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
            ..Default::default()
        }
    }

    pub fn bind(&mut self, params: RTCRtpParameters, o: Options) {
        let codec = &params.codecs[0];
        self.clock_rate = codec.capability.clock_rate;
        self.max_bitrate = o.max_bitrate;
        self.mime = codec.capability.mime_type.to_lowercase();

        if self.mime.starts_with("audio/") {
            self.codec_type = RTPCodecType::Audio;
            self.bucket = Some(Bucket::new(self.audio_pool_len));
        } else if self.mime.starts_with("video/") {
            self.codec_type = RTPCodecType::Video;
            self.bucket = Some(Bucket::new(self.video_pool_len));
        } else {
            self.codec_type = RTPCodecType::Unspecified;
        }

        for ext in &params.header_extensions {
            if ext.uri == extmap::TRANSPORT_CC_URI {
                self.twcc_ext = ext.id as u8;
                break;
            }
        }

        match self.codec_type {
            RTPCodecType::Video => {
                for feedback in &codec.capability.rtcp_feedback {
                    match feedback.typ.clone().as_str() {
                        webrtc_rtp::TYPE_RTCP_FB_GOOG_REMB => {
                            self.remb = true;
                        }
                        webrtc_rtp::TYPE_RTCP_FB_TRANSPORT_CC => {
                            self.twcc = true;
                        }
                        webrtc_rtp::TYPE_RTCP_FB_NACK => {
                            self.nack = true;
                            self.nacker = Some(NackQueue::new());
                        }
                        _ => {}
                    }
                }
            }
            RTPCodecType::Audio => {
                for ext in &params.header_extensions {
                    if ext.uri == extmap::AUDIO_LEVEL_URI {
                        self.audio_level = true;
                        self.audio_ext = ext.id as u8;
                    }
                }
            }

            _ => {}
        }

        for pp in self.pending_packets.clone() {
            self.calc(&pp.packet[..], pp.arrival_time as i64);
        }
        self.pending_packets.clear();
        self.bound = true;
    }

    pub async fn read_extended(&mut self) -> Result<ExtPacket> {
        loop {
            if self.closed {
                return Err(Error::ErrIOEof.into());
            }

            if self.ext_packets.len() > 0 {
                let ext_pkt = self.ext_packets.pop_front().unwrap();
                return Ok(ext_pkt);
            }

            sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn on_close(&self, f: OnCloseFn) {
        let mut handler = self.on_close_handler.lock().await;
        *handler = Some(f);
    }

    pub fn calc(&mut self, pkt: &[u8], arrival_time: i64) {
        let sn = BigEndian::read_u16(&pkt[2..4]);

        if self.stats.packet_count == 0 {
            self.base_sn = sn;
            self.max_seq_no = sn;
            self.last_report = arrival_time;
        } else if (sn - self.max_seq_no) & 0x8000 == 0 {
            if sn < self.max_seq_no {
                self.cycles += MAX_SEQUENCE_NUMBER;
            }
            if self.nack {
                let diff = sn - self.max_seq_no;

                for i in 1..diff {
                    let ext_sn: u32;

                    let msn = sn - i;
                    if msn > self.max_seq_no && (msn & 0x8000) > 0 && self.max_seq_no & 0x8000 == 0
                    {
                        ext_sn = (self.cycles - MAX_SEQUENCE_NUMBER) | msn as u32;
                    } else {
                        ext_sn = self.cycles | msn as u32;
                    }

                    if let Some(nacker) = self.nacker.as_mut() {
                        nacker.push(ext_sn);
                    }
                }
            }
            self.max_seq_no = sn;
        } else if self.nack && ((sn - self.max_seq_no) & 0x8000 > 0) {
            let ext_sn: u32;

            if sn > self.max_seq_no && (sn & 0x8000) > 0 && self.max_seq_no & 0x8000 == 0 {
                ext_sn = (self.cycles - MAX_SEQUENCE_NUMBER) | sn as u32;
            } else {
                ext_sn = self.cycles | sn as u32;
            }
            if let Some(nacker) = self.nacker.as_mut() {
                nacker.remove(ext_sn);
            }
        }

        let mut packet: Packet = Packet::default();

        if let Some(bucket) = &mut self.bucket {
            let rv = bucket.add_packet(pkt, sn, sn == self.max_seq_no);

            match rv {
                Ok(data) => match Packet::unmarshal(&mut &data[..]) {
                    Err(_) => {
                        return;
                    }
                    Ok(p) => {
                        packet = p;
                    }
                },
                Err(_) => {
                    //  if Error::ErrRTXPacket.equal(&rv) {
                    return;
                }
            }
        }

        self.stats.total_byte += pkt.len() as u64;
        self.bitrate_helper += pkt.len() as u64;
        self.stats.packet_count += 1;

        let mut ep = ExtPacket {
            head: sn == self.max_seq_no,
            cycle: self.cycles,
            packet: packet.clone(),
            arrival: arrival_time,
            key_frame: false,
            ..Default::default()
        };

        match self.mime.as_str() {
            "video/vp8" => {
                let mut vp8_packet = helpers::VP8::default();
                if let Err(_) = vp8_packet.unmarshal(&packet.payload[..]) {
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

        if self.min_packet_probe < 25 {
            if sn < self.base_sn {
                self.base_sn = sn
            }

            if self.mime == "video/vp8" {
                let pld = ep.payload;
                let mtl = self.max_temporal_layer;
                if mtl < pld.tid as i32 {
                    self.max_temporal_layer = pld.tid as i32;
                }
            }

            self.min_packet_probe += 1;
        }

        self.ext_packets.push_back(ep);

        // if first time update or the timestamp is later (factoring timestamp wrap around)
        let latest_timestamp = self.latest_timestamp;
        let latest_timestamp_in_nanos_since_epoch = self.latest_timestamp_time;
        if (latest_timestamp_in_nanos_since_epoch == 0)
            || is_later_timestamp(packet.header.timestamp, latest_timestamp)
        {
            self.latest_timestamp = packet.header.timestamp;
            self.latest_timestamp_time = arrival_time;
        }

        let arrival = arrival_time as f64 / 1e6 * (self.clock_rate as f64 / 1e3);
        let transit = arrival - packet.header.timestamp as f64;
        if self.last_transit != 0 {
            let mut d = transit - self.last_transit as f64;
            if d < 0.0 {
                d *= -1.0;
            }

            self.stats.jitter += (d - self.stats.jitter) / 16 as f64;
        }
        self.last_transit = transit as u32;

        if self.twcc {
            if let Some(ext) = packet.header.get_extension(self.twcc_ext) {
                if ext.len() > 1 {
                    // feedback
                }
            }
        }

        if self.audio_level {
            if let Some(ext) = packet.header.get_extension(self.audio_ext) {
                let rv = AudioLevelExtension::unmarshal(&mut &ext[..]);

                if let Ok(data) = rv {}

                // audio_ext
            }
        }

        let diff = arrival_time - self.last_report;

        if let Some(nacker) = &self.nacker {
            let rv = self.build_nack_packet();
        }

        if diff as f64 >= REPORT_DELTA {
            let br = 8 * self.bitrate_helper * REPORT_DELTA as u64 / diff as u64;
            self.bitrate = br;
            self.last_report = arrival_time;
            self.bitrate_helper = 0;
        }
    }

    fn build_nack_packet(&mut self) -> Vec<Box<dyn RtcpPacket>> {
        let mut pkts: Vec<Box<dyn RtcpPacket>> = Vec::new();

        if self.nacker == None {
            return pkts;
        }
        let (nacks, ask_key_frame) = self
            .nacker
            .as_mut()
            .unwrap()
            .pairs(self.cycles | self.max_seq_no as u32);

        let mut nacks_len: usize = 0;
        if nacks.is_some() {
            nacks_len = nacks.as_ref().unwrap().len();
        }

        if nacks_len > 0 || ask_key_frame {
            if nacks_len > 0 {
                let pkt = TransportLayerNack {
                    media_ssrc: self.media_ssrc,
                    nacks: nacks.unwrap(),
                    ..Default::default()
                };
                pkts.push(Box::new(pkt));
            }

            if ask_key_frame {
                let pkt = PictureLossIndication {
                    media_ssrc: self.media_ssrc,
                    ..Default::default()
                };
                pkts.push(Box::new(pkt));
            }
        }

        pkts
    }

    fn build_remb_packet(&mut self) -> ReceiverEstimatedMaximumBitrate {
        let mut br = self.bitrate;

        if self.stats.lost_rate < 0.02 {
            br = (br as f64 * 1.09) as u64 + 2000;
        }

        if self.stats.lost_rate > 0.1 {
            br = (br as f64 * (1.0 - 0.5 * self.stats.lost_rate) as f64) as u64;
        }

        if br > self.max_bitrate {
            br = self.max_bitrate;
        }

        if br < 100000 {
            br = 100000;
        }

        self.stats.total_byte = 0;

        return ReceiverEstimatedMaximumBitrate {
            bitrate: br as f32,
            ssrcs: vec![self.media_ssrc],
            ..Default::default()
        };
    }

    fn build_reception_report(&mut self) -> ReceptionReport {
        let ext_max_seq = self.cycles | self.max_seq_no as u32;
        let expected = ext_max_seq - self.base_sn as u32 + 1;
        let mut lost: u32 = 0;

        if self.stats.packet_count < expected && self.stats.packet_count != 0 {
            lost = expected - self.stats.packet_count;
        }

        let expected_interval = expected - self.stats.last_expected;
        self.stats.last_expected = expected;

        let received_interval = self.stats.packet_count - self.stats.last_received;
        self.stats.last_received = self.stats.packet_count;

        let lost_interval = expected_interval - received_interval;

        self.stats.lost_rate = lost_interval as f32 / expected_interval as f32;

        let mut frac_lost: u8 = 0;
        if expected_interval != 0 && lost_interval > 0 {
            frac_lost = ((lost_interval << 8) / expected_interval) as u8;
        }

        let mut dlsr: u32 = 0;

        if self.last_sr_recv != 0 {
            let delay_ms = ((Instant::now().elapsed().subsec_nanos() - self.last_sr_recv as u32)
                as f64
                / 1e6) as u32;

            dlsr = ((delay_ms as f32 / 1e3) as u32) << 16;

            dlsr |= (delay_ms as f32 % 1e3) as u32 * 65536 / 1000;
        }

        ReceptionReport {
            ssrc: self.media_ssrc,
            fraction_lost: frac_lost,
            total_lost: lost,
            last_sequence_number: ext_max_seq,
            jitter: self.stats.jitter as u32,
            last_sender_report: self.last_srrtp_time >> 16,
            delay: dlsr,
        }
    }

    pub fn set_sender_report_data(&mut self, rtp_time: u32, ntp_time: u64) {
        self.last_srrtp_time = rtp_time;
        self.last_srntp_time = ntp_time;
        self.last_sr_recv = Instant::now().elapsed().subsec_nanos() as i64;
    }

    fn get_rtcp(&mut self) -> Vec<Box<dyn RtcpPacket>> {
        let mut rtcp_packets: Vec<Box<dyn RtcpPacket>> = Vec::new();
        rtcp_packets.push(Box::new(self.build_reception_report()));

        if self.remb && !self.twcc {
            rtcp_packets.push(Box::new(self.build_remb_packet()));
        }

        rtcp_packets
    }

    pub fn get_packet(&self, buff: &mut [u8], sn: u16) -> Result<usize> {
        if self.closed {
            return Err(Error::ErrIOEof.into());
        }

        if let Some(bucket) = &self.bucket {
            return bucket.get_packet(buff, sn);
        }

        Ok(0)
    }

    fn bitrate(&self) -> u64 {
        self.bitrate
    }

    fn max_temporal_layer(&self) -> i32 {
        self.max_temporal_layer
    }

    pub async fn on_transport_wide_cc(&self, f: OnTransportWideCCFn) {
        let mut handler = self.on_transport_wide_cc_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_feedback_callback(&self, f: OnFeedbackCallBackFn) {
        let mut handler = self.on_feedback_callback_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_audio_level(&self, f: OnAudioLevelFn) {
        let mut handler = self.on_audio_level_handler.lock().await;
        *handler = Some(f);
    }

    fn get_media_ssrc(&self) -> u32 {
        self.media_ssrc
    }

    fn get_clock_rate(&self) -> u32 {
        self.clock_rate
    }

    pub fn get_sender_report_data(&self) -> (u32, u64, i64) {
        (
            self.last_srrtp_time,
            self.last_srntp_time,
            self.last_sr_recv,
        )
    }

    pub fn get_status(&self) -> Stats {
        self.stats.clone()
    }

    fn get_latest_timestamp(&self) -> (u32, i64) {
        (self.latest_timestamp, self.latest_timestamp_time)
    }
}

impl BufferIO for Buffer {
    // Write adds a RTP Packet, out of order, new packet may be arrived later
    fn write(&mut self, pkt: &[u8]) -> Result<u32> {
        if !self.bound {
            self.pending_packets.push(PendingPackets {
                arrival_time: Instant::now().elapsed().subsec_nanos() as u64,
                packet: Vec::from(pkt),
            });

            return Ok(0);
        }

        self.calc(pkt, Instant::now().elapsed().subsec_nanos() as i64);

        Ok(0)
    }

    fn read(&mut self, buff: &mut [u8]) -> Result<usize> {
        if self.closed {
            return Err(Error::ErrIOEof.into());
        }

        let mut n: usize = 0;

        if self.pending_packets.len() > self.last_packet_read as usize {
            if buff.len()
                < self
                    .pending_packets
                    .get(self.last_packet_read as usize)
                    .unwrap()
                    .packet
                    .len()
            {
                return Err(Error::ErrBufferTooSmall.into());
            }

            let packet = &self
                .pending_packets
                .get(self.last_packet_read as usize)
                .unwrap()
                .packet;

            n = packet.len();

            buff.copy_from_slice(&packet[..]);
            return Ok(n);
        }

        Ok(n)
    }
    fn close(&mut self) -> Result<()> {
        if self.bucket.is_some() && self.codec_type == RTPCodecType::Video {}

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
