use rand::Rng;
use rtcp::raw_packet::RawPacket;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

const BASE_SEQUENCE_NUMBER_OFFSET: i64 = 8;
const PACKET_STATUS_COUNT_OFFSET: i64 = 10;
const REFERENCE_TIME_OFFSET: i64 = 12;

const TCC_REPORT_DELTA: f64 = 1e8;
const TCC_REPORT_DELTA_AFTER_MARK: f64 = 50e6;

pub type OnResponderFeedbackFn =
    Box<dyn (FnMut(RawPacket) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

#[derive(Default, Clone)]
pub struct RtpExtInfo {
    ext_tsn: u32,
    timestamp: i64,
}

// Responder will get the transport wide sequence number from rtp
// extension header, and reply with the rtcp feedback message
// according to:
// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01

#[derive(Clone)]
pub struct Responder {
    ext_info: Arc<Mutex<Vec<RtpExtInfo>>>,
    last_report: i64,
    cycles: u32,
    last_ext_sn: u32,
    pkt_ctn: u8,
    last_sn: u16,
    last_ext_info: u16,
    m_ssrc: u32,
    s_ssrc: u32,

    len: u16,
    delta_len: u16,
    payload: Vec<u8>,
    deltas: Vec<u8>,
    chunk: u16,

    on_feedback_handler: Arc<Mutex<Option<OnResponderFeedbackFn>>>,
}

impl Responder {
    fn new(ssrc: u32) -> Self {
        let mut rng = rand::thread_rng();

        Responder {
            ext_info: Arc::new(Mutex::new(Vec::new())),
            last_report: 0,
            cycles: 0,
            last_ext_sn: 0,
            pkt_ctn: 0,
            last_sn: 0,
            last_ext_info: 0,

            s_ssrc: rng.gen::<u32>(),
            m_ssrc: ssrc,

            len: 0,
            delta_len: 0,
            payload: Vec::new(),
            deltas: Vec::new(),
            chunk: 0,
            on_feedback_handler: Arc::new(Mutex::new(None)),
        }
    }

    async fn push(&mut self, sn: u16, time_ns: i64, marker: bool) {
        if sn < 0x0fff && (self.last_sn & 0xffff) > 0xf000 {
            self.cycles += 1 << 16
        }

        let mut ext_infos = self.ext_info.lock().await;

        let ext_info = RtpExtInfo {
            ext_tsn: self.cycles | sn as u32,
            timestamp: time_ns / 100,
        };

        ext_infos.push(ext_info);

        if self.last_report == 0 {
            self.last_report = time_ns;
        }

        self.last_sn = sn;
        let delta = time_ns - self.last_report;

        if ext_infos.len() > 20
            && self.m_ssrc != 0
            && (delta as f64 >= TCC_REPORT_DELTA 
                || ext_infos.len() > 100
                || (marker && delta as f64 >= TCC_REPORT_DELTA_AFTER_MARK))
        {}
    }

    async fn build_transport_cc_packet(&mut self) -> Option<RawPacket> {
        let ext_infos = self.ext_info.lock().await;

        if ext_infos.len() == 0 {
            return None;
        }

        None
    }
}
