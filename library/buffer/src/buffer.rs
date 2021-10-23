use rtp::packet::Packet;
use webrtc::media::rtp::rtp_codec::RTPCodecType;

use std::collections::VecDeque;

use crate::{bucket::Bucket, nack::NackQueue};
#[derive(Debug, Eq, PartialEq, Default, Clone)]
struct PendingPackets {
    arrival_time: u64,
    packet: Vec<u8>,
}
#[derive(Debug, Eq, PartialEq, Default, Clone)]
struct ExtPacket {
    head: bool,
    cycle: u32,
    arrival: i64,
    packet: Packet,
    key_frame: bool,
}
#[derive(Debug, PartialEq, Default, Clone)]
struct Stats {
    last_expected: u32,
    last_received: u32,
    lost_rate: f32,
    packet_count: u32, // Number of packets received from this source.
    jitter: f64, // An estimate of the statistical variance of the RTP data packet inter-arrival time.
    total_byte: u64,
}
#[derive(Debug, Eq, PartialEq, Default, Clone)]
struct Options {
    max_bitrate: u64,
}
#[derive(Debug, PartialEq, Default, Clone)]
struct Buffer {
    bucket: Bucket,
    nacker: NackQueue,

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
    mime: String,

    // supported feedbacks
    remb: bool,
    nack: bool,
    twcc: bool,
    audio_level: bool,

    min_packet_probe: i32,
    last_packet_read: i32,

    max_temporal_layer: i32,
    bitrate: u64,
    bitrate_helper: u64,
    last_rtntp_time: u64,
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

    //callbacks
    on_close: fn(),
    on_audio_level: fn(level: u8),
    feedback_cb: fn(packets: &[Packet]),
    feedback_twcc: fn(sn: u16, time_ns: i64, marker: bool),
}

impl Buffer {
    fn new(ssrc: u32) -> Self {
        Self {
            media_ssrc: ssrc,
            ..Default::default()
        }
    }
}
