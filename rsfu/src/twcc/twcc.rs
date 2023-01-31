use super::errors::*;
use anyhow::Result;
use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use rand::Rng;
use rtcp::header::Header;
use rtcp::raw_packet::RawPacket;
use std::collections::VecDeque;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc_util::Marshal;

use rtcp::header;
use rtcp::header::PacketType;
use rtcp::transport_feedbacks::transport_layer_cc::SymbolSizeTypeTcc;
use rtcp::transport_feedbacks::transport_layer_cc::SymbolTypeTcc;

use rtcp::packet::Packet as RtcpPacket;

// const BASE_SEQUENCE_NUMBER_OFFSET: i64 = 8;
// const PACKET_STATUS_COUNT_OFFSET: i64 = 10;
// const REFERENCE_TIME_OFFSET: i64 = 12;

const TCC_REPORT_DELTA: f64 = 1e8;
const TCC_REPORT_DELTA_AFTER_MARK: f64 = 50e6;

pub type OnResponderFeedbackFn = Box<
    dyn (FnMut(
            Box<dyn RtcpPacket + Send + Sync>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

#[derive(Default, Clone)]
pub struct RtpExtInfo {
    pub ext_tsn: u32,
    pub timestamp: i64,
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
    pub last_ext_sn: u32,
    pub pkt_ctn: u8,
    last_sn: u16,
    pub last_ext_info: u16,
    pub m_ssrc: u32,
    pub s_ssrc: u32,

    pub len: u16,
    pub delta_len: usize,
    pub payload: Vec<u8>,
    pub deltas: Vec<u8>,
    chunk: u16,

    on_feedback_handler: Arc<Mutex<Option<OnResponderFeedbackFn>>>,
}

fn set_n_bits_of_u16(src: u16, size: u16, start_index: u16, val: u16) -> u16 {
    if start_index + size > 16 {
        return 0;
    }
    // truncate val to size bits
    let val2 = val & (1 << size) - 1;
    return src | (val2 << (16 - size - start_index));
}

impl Responder {
    pub fn new(ssrc: u32) -> Self {
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

    pub async fn push(&mut self, sn: u16, time_ns: i64, marker: bool) {
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

    pub async fn on_feedback(&mut self, f: OnResponderFeedbackFn) {
        let mut handler = self.on_feedback_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn build_transport_cc_packet(&mut self) -> Result<RawPacket> {
        let ext_info = Arc::clone(&self.ext_info);
        let mut ext_infos = ext_info.lock().await;

        if ext_infos.len() == 0 {
            return Err(Error::ErrExtInfoEmpty.into());
        }

        ext_infos.sort_by(|a, b| {
            return b.ext_tsn.cmp(&a.ext_tsn);
        });

        let mut tcc_pkts: Vec<RtpExtInfo> = Vec::new();

        for ext_info in ext_infos.iter() {
            if ext_info.ext_tsn < self.last_ext_sn {
                continue;
            }

            if self.last_ext_sn != 0 {
                for idx in self.last_ext_sn + 1..ext_info.ext_tsn {
                    tcc_pkts.push(RtpExtInfo {
                        ext_tsn: idx,
                        ..Default::default()
                    })
                }
            }
            self.last_ext_sn = ext_info.ext_tsn;
            tcc_pkts.push(ext_info.clone());
        }

        ext_infos.clear();

        let mut first_recv = false;
        let mut same = true;
        let mut timestamp: i64 = 0;

        let mut last_status = SymbolTypeTcc::PacketReceivedWithoutDelta;
        let mut max_status = SymbolTypeTcc::PacketNotReceived;

        let mut status_list: VecDeque<SymbolTypeTcc> = VecDeque::new();

        for stat in &tcc_pkts {
            let mut status = SymbolTypeTcc::PacketNotReceived;

            if stat.timestamp != 0 {
                let delta: i64;
                if !first_recv {
                    first_recv = true;

                    let ref_time = stat.timestamp / 64e3 as i64;

                    timestamp = ref_time * 64e3 as i64;

                    self.write_header(
                        tcc_pkts[0].ext_tsn as u16,
                        tcc_pkts.len() as u16,
                        ref_time as u32,
                    )?;

                    self.pkt_ctn += 1;
                }

                delta = (stat.timestamp - timestamp) / 250;

                if delta < 0 || delta > 255 {
                    status = SymbolTypeTcc::PacketReceivedLargeDelta;
                    let mut r_delta = delta as i16;

                    if r_delta as i64 != delta {
                        if r_delta > 0 {
                            r_delta = std::i16::MAX;
                        } else {
                            r_delta = std::i16::MIN;
                        }
                    }
                    self.write_delta(status, r_delta as u16)?;
                } else {
                    status = SymbolTypeTcc::PacketReceivedSmallDelta;
                    self.write_delta(status, delta as u16)?;
                }

                timestamp = stat.timestamp;
            }
            if same
                && status != last_status
                && last_status != SymbolTypeTcc::PacketReceivedWithoutDelta
            {
                if status_list.len() > 7 {
                    self.write_run_length_chunk(last_status as u16, status_list.len() as u16)?;
                    status_list.clear();
                    //it is overwritten before read
                    //last_status = SymbolTypeTcc::PacketReceivedWithoutDelta;
                    max_status = SymbolTypeTcc::PacketNotReceived;
                    same = true;
                } else {
                    same = false;
                }
            }

            status_list.push_back(status);

            if status as u16 > max_status as u16 {
                max_status = status;
            }
            last_status = status;

            if !same
                && max_status == SymbolTypeTcc::PacketReceivedLargeDelta
                && status_list.len() > 6
            {
                for i in 0..7 {
                    let stats = status_list.pop_front().unwrap() as u16;
                    self.create_status_symbol_chunk(SymbolSizeTypeTcc::TwoBit, stats, i);
                }
                self.write_status_symbol_chunk(SymbolSizeTypeTcc::TwoBit)?;

                last_status = SymbolTypeTcc::PacketReceivedWithoutDelta;
                max_status = SymbolTypeTcc::PacketNotReceived;
                same = true;

                for i in 0..status_list.len() {
                    status = status_list.get(i).unwrap().clone();
                    if status as u16 > max_status as u16 {
                        max_status = status;
                    }

                    if same
                        && last_status != SymbolTypeTcc::PacketReceivedWithoutDelta
                        && status != last_status
                    {
                        same = false;
                    }
                    last_status = status;
                }
            } else if !same && status_list.len() > 13 {
                for i in 0..14 {
                    let symbol = status_list.pop_front().unwrap() as u16;
                    self.create_status_symbol_chunk(SymbolSizeTypeTcc::OneBit, symbol, i);
                }

                self.write_status_symbol_chunk(SymbolSizeTypeTcc::OneBit)?;
                last_status = SymbolTypeTcc::PacketReceivedWithoutDelta;
                max_status = SymbolTypeTcc::PacketNotReceived;
                same = true;
            }
        }
        if status_list.len() > 0 {
            if same {
                self.write_run_length_chunk(last_status as u16, status_list.len() as u16)?;
            } else if max_status == SymbolTypeTcc::PacketReceivedLargeDelta {
                for i in 0..status_list.len() {
                    let symbol = status_list.pop_front().unwrap() as u16;
                    self.create_status_symbol_chunk(SymbolSizeTypeTcc::TwoBit, symbol, i as u16);
                }
                self.write_status_symbol_chunk(SymbolSizeTypeTcc::TwoBit)?;
            } else {
                for i in 0..status_list.len() {
                    let symbol = status_list.pop_front().unwrap() as u16;
                    self.create_status_symbol_chunk(SymbolSizeTypeTcc::OneBit, symbol, i as u16);
                }
                self.write_status_symbol_chunk(SymbolSizeTypeTcc::OneBit)?;
            }
        }

        let mut p_len = self.len as usize + self.delta_len + 4;
        let pad = p_len != 0;
        let mut pad_size: u8 = 0;

        while p_len % 4 != 0 {
            pad_size += 1;
            p_len += 1;
        }

        let hdr = Header {
            padding: pad,
            length: p_len as u16 / 4 - 1,
            count: header::FORMAT_TCC,
            packet_type: PacketType::TransportSpecificFeedback,
        };

        let mut raw_packet_data: Vec<u8> = Vec::new();

        hdr.marshal_to(&mut raw_packet_data[..])?; //header

        raw_packet_data.write(&self.payload[..])?;
        raw_packet_data.write(&self.deltas[..])?;

        if pad {
            raw_packet_data.write_u8(pad_size)?;
        }

        self.delta_len = 0;

        let bytes = Bytes::copy_from_slice(&raw_packet_data[..]);

        Ok(RawPacket(bytes))
    }

    pub fn write_header(&mut self, b_sn: u16, packet_count: u16, ref_time: u32) -> Result<()> {
        self.payload.write_u32::<BigEndian>(self.s_ssrc)?;
        self.payload.write_u32::<BigEndian>(self.m_ssrc)?;
        self.payload.write_u16::<BigEndian>(b_sn)?;
        self.payload.write_u16::<BigEndian>(packet_count)?;
        self.payload
            .write_u32::<BigEndian>(ref_time << 8 | (self.pkt_ctn as u32))?;

        Ok(())
    }

    pub fn write_run_length_chunk(&mut self, symbol: u16, run_length: u16) -> Result<()> {
        /*
           +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
           |T| S |       Run Length        |
           +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        */
        self.payload
            .write_u16::<BigEndian>(symbol << 13 | run_length)?;

        // BigEndian::write_u16(
        //     &mut self.payload[self.len as usize..],
        //     symbol << 13 | run_length,
        // );

        Ok(())
    }

    pub fn create_status_symbol_chunk(
        &mut self,
        symbol_size: SymbolSizeTypeTcc,
        symbol: u16,
        i: u16,
    ) {
        /*
            +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
            |T|S|       symbol list         |
            +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        */
        let num_of_bits = symbol_size as u16 + 1;
        self.chunk = set_n_bits_of_u16(self.chunk, num_of_bits, num_of_bits * i + 2, symbol);
    }

    pub fn write_status_symbol_chunk(&mut self, symbol_size: SymbolSizeTypeTcc) -> Result<()> {
        self.chunk = set_n_bits_of_u16(self.chunk, 1, 0, 1);
        self.chunk = set_n_bits_of_u16(self.chunk, 1, 1, symbol_size as u16);

        self.payload.write_u16::<BigEndian>(self.chunk)?;

        self.chunk = 0;
        self.len += 2;

        Ok(())
    }

    pub fn write_delta(&mut self, delta_type: SymbolTypeTcc, delta: u16) -> Result<()> {
        if delta_type == SymbolTypeTcc::PacketReceivedSmallDelta {
            // let deltas_capacity = self.deltas.capacity();
            // if deltas_capacity <= self.delta_len {
            //     self.deltas.resize(deltas_capacity + self.delta_len + 1, 0);
            // }
            //self.deltas[self.delta_len] = delta as u8;

            self.deltas.write_u8(delta as u8)?;
            self.delta_len += 1;
            return Ok(());
        }

        self.deltas.write_u16::<BigEndian>(delta)?;
        self.delta_len += 2;

        Ok(())
    }
}
