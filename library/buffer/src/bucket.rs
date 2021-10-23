use std::io::Write;

use crate::errors::*;

use anyhow::Result;
use byteorder::{BigEndian, ByteOrder, WriteBytesExt};

const MAX_PACKET_SIZE: usize = 1500;

fn distance(arg1: u16, arg2: u16) -> u16 {
    if arg1 < arg2 {
        65535 - arg2 + arg1
    } else {
        arg1 - arg2
    }
}
#[derive(Debug, Eq, PartialEq, Default, Clone)]
pub struct Bucket {
    buf: Vec<u8>,
    src: Vec<u8>,

    init: bool,
    step: i32,
    headSN: u16,
    maxSteps: i32,
}

impl Bucket {
    pub fn new(buf: &[u8]) -> Self {
        Self {
            buf: buf.to_vec(),
            src: Vec::new(),
            init: false,
            step: 0,
            headSN: 0,
            maxSteps: (buf.len() / MAX_PACKET_SIZE) as i32 - 1,
        }
    }

    pub fn add_packet(&mut self, pkt: &[u8], sn: u16, latest: bool) -> Result<Vec<u8>> {
        if !self.init {
            self.headSN = sn - 1;
            self.init = true;
        }

        if !latest {
            return self.set(sn, pkt);
        }

        let diff: u16 = distance(sn, self.headSN);
        self.headSN = sn;

        for _ in 1..diff {
            self.step += 1;
            if self.step >= self.maxSteps {
                self.step = 0;
            }
        }

        self.push(pkt)
    }

    fn push(&mut self, pkt: &[u8]) -> Result<Vec<u8>> {
        let pkt_len = pkt.len();
        let pkt_len_idx = self.step as usize * MAX_PACKET_SIZE;
        if self.buf.capacity() < pkt_len_idx + pkt_len {
            return Err(Error::ErrBufferTooSmall.into());
        }
        BigEndian::write_u16(&mut self.buf[pkt_len_idx..], pkt_len as u16);

        let off = pkt_len_idx + 2;

        self.buf[off..off + pkt_len].copy_from_slice(pkt);

        self.step += 1;

        if self.step > self.maxSteps {
            self.step = 0;
        }

        Ok(self.buf[off..off + pkt_len].to_vec())
    }

    pub fn get(&mut self, sn: u16) -> Option<Vec<u8>> {
        let diff: u16 = distance(self.headSN, sn);

        let mut pos = self.step - (diff + 1) as i32;
        if pos < 0 {
            if pos * -1 > self.maxSteps + 1 {
                return None;
            }
            pos = self.maxSteps + pos + 1;
        }

        let off = pos as usize * MAX_PACKET_SIZE;

        if off > self.buf.len() {
            return None;
        }

        if BigEndian::read_u16(&self.buf[off as usize + 4..]) != sn {
            return None;
        }

        let size = BigEndian::read_u16(&self.buf[off as usize..]);

        Some(self.buf[off + 2..off + 2 + size as usize].to_vec())
    }

    fn set(&mut self, sn: u16, pkt: &[u8]) -> Result<Vec<u8>> {
        let diff: u16 = distance(self.headSN, sn);
        if diff >= self.maxSteps as u16 + 1 {
            return Err(Error::ErrPacketTooOld.into());
        }
        let mut pos = self.step - (diff + 1) as i32;
        if pos < 0 {
            pos = self.maxSteps + pos + 1
        }

        let off = pos * MAX_PACKET_SIZE as i32;
        if off > self.buf.len() as i32 || off < 0 {
            return Err(Error::ErrPacketTooOld.into());
        }

        if BigEndian::read_u16(&self.buf[off as usize + 4..]) == sn {
            return Err(Error::ErrRTXPacket.into());
        }

        let pkt_len = pkt.len();

        BigEndian::write_u16(&mut self.buf[off as usize..], pkt_len as u16);

        self.buf[off as usize + 2..off as usize + 2 + pkt_len].copy_from_slice(pkt);

        Ok(pkt.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use byteorder::{BigEndian, ByteOrder, WriteBytesExt};

    #[test]
    fn test_bigendian() {
        let mut datas = vec![0; 30];

        let mut src = vec![2, 3, 4];

        let length = src.len();

        datas[2..length + 2].copy_from_slice(&src);

        //    let rv = BigEndian::read_u16(&datas[1..]);

        //datas.write_u16::<BigEndian>(14);

        // BigEndian::write_u16(&mut datas[2 as usize..], 23 as u16);

        // print!("data is :{:02x}\n", rv);

        for i in datas {
            println!("data is-- :{:02x}\n", i);
        }
    }
}
