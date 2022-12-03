use crate::buffer::errors::*;
use anyhow::Result;

// VP8 is a helper to get temporal data from VP8 packet header
/*
    VP8 Payload Descriptor
            0 1 2 3 4 5 6 7                      0 1 2 3 4 5 6 7
            +-+-+-+-+-+-+-+-+                   +-+-+-+-+-+-+-+-+
            |X|R|N|S|R| PID | (REQUIRED)        |X|R|N|S|R| PID | (REQUIRED)
            +-+-+-+-+-+-+-+-+                   +-+-+-+-+-+-+-+-+
        X:  |I|L|T|K| RSV   | (OPTIONAL)   X:   |I|L|T|K| RSV   | (OPTIONAL)
            +-+-+-+-+-+-+-+-+                   +-+-+-+-+-+-+-+-+
        I:  |M| PictureID   | (OPTIONAL)   I:   |M| PictureID   | (OPTIONAL)
            +-+-+-+-+-+-+-+-+                   +-+-+-+-+-+-+-+-+
        L:  |   TL0PICIDX   | (OPTIONAL)        |   PictureID   |
            +-+-+-+-+-+-+-+-+                   +-+-+-+-+-+-+-+-+
        T/K:|TID|Y| KEYIDX  | (OPTIONAL)   L:   |   TL0PICIDX   | (OPTIONAL)
            +-+-+-+-+-+-+-+-+                   +-+-+-+-+-+-+-+-+
        T/K:|TID|Y| KEYIDX  | (OPTIONAL)
            +-+-+-+-+-+-+-+-+
*/
#[derive(Debug, Eq, PartialEq, Default, Clone, Copy)]
pub struct VP8 {
    pub temporal_supported: bool,
    // Optional Header
    pub picture_id: u16, /* 8 or 16 bits, picture ID */
    pub picture_id_idx: i32,
    pub mbit: bool,
    pub tl0_picture_idx: u8, /* 8 bits temporal level zero index */
    pub tlz_idx: i32,

    // Optional Header If either of the T or K bits are set to 1,
    // the TID/Y/KEYIDX extension field MUST be present.
    pub tid: u8, /* 2 bits temporal layer idx*/
    // IsKeyFrame is a helper to detect if current packet is a keyframe
    pub is_key_frame: bool,
}

impl VP8 {
    pub fn unmarshal(&mut self, payload: &[u8]) -> Result<()> {
        let payload_len = payload.len();
        if payload_len == 0 {
            return Err(Error::ErrNilPacket.into());
        }

        let mut idx: usize = 0;
        let s = payload[idx] & 0x10 > 0;
        if payload[idx] & 0x80 > 0 {
            idx += 1;

            if payload_len < idx + 1 {
                return Err(Error::ErrShortPacket.into());
            }

            self.temporal_supported = payload[idx] & 0x20 > 0;
            let k = payload[idx] & 0x10 > 0;
            let l = payload[idx] & 0x40 > 0;

            // Check for PictureID
            if payload[idx] & 0x80 > 0 {
                idx += 1;
                if payload_len < idx + 1 {
                    return Err(Error::ErrShortPacket.into());
                }
                self.picture_id_idx = idx as i32;
                let pid = payload[idx] & 0x7f;
                // Check if m is 1, then Picture ID is 15 bits
                if payload[idx] & 0x80 > 0 {
                    idx += 1;
                    if payload_len < idx + 1 {
                        return Err(Error::ErrShortPacket.into());
                    }
                    self.mbit = true;

                    self.picture_id = ((pid as u16) << 8) | payload[idx] as u16;
                } else {
                    self.picture_id = pid as u16;
                }
            }

            // Check if TL0PICIDX is present
            if l {
                idx += 1;
                if payload_len < idx + 1 {
                    return Err(Error::ErrShortPacket.into());
                }
                self.tlz_idx = idx as i32;

                if idx >= payload_len {
                    return Err(Error::ErrShortPacket.into());
                }
                self.tl0_picture_idx = payload[idx];
            }

            if self.temporal_supported || k {
                idx += 1;
                if payload_len < idx + 1 {
                    return Err(Error::ErrShortPacket.into());
                }
                self.tid = (payload[idx] & 0xc0) >> 6;
            }

            if idx >= payload_len {
                return Err(Error::ErrShortPacket.into());
            }
            idx += 1;
            if payload_len < idx + 1 {
                return Err(Error::ErrShortPacket.into());
            }
            // Check is packet is a keyframe by looking at P bit in vp8 payload
            self.is_key_frame = payload[idx] & 0x01 == 0 && s;
        } else {
            idx += 1;
            if payload_len < idx + 1 {
                return Err(Error::ErrShortPacket.into());
            }
            // Check is packet is a keyframe by looking at P bit in vp8 payload
            self.is_key_frame = payload[idx] & 0x01 == 0 && s;
        }

        Ok(())
    }
}

pub fn is_h264_keyframe(payload: &[u8]) -> bool {
    if payload.len() < 1 {
        return false;
    }
    let nalu = payload[0] & 0x1F;
    if nalu == 0 {
        // reserved
        return false;
    } else if nalu <= 23 {
        // simple NALU
        return nalu == 5;
    } else if nalu == 24 || nalu == 25 || nalu == 26 || nalu == 27 {
        // STAP-A, STAP-B, MTAP16 or MTAP24
        let mut i = 1;
        if nalu == 25 || nalu == 26 || nalu == 27 {
            // skip DON
            i += 2;
        }
        while i < payload.len() {
            if i + 2 > payload.len() {
                return false;
            }
            let length = ((payload[i] as u16) << 8) | payload[i + 1] as u16;
            i += 2;
            if i + length as usize > payload.len() {
                return false;
            }
            let mut offset = 0;
            if nalu == 26 {
                offset = 3;
            } else if nalu == 27 {
                offset = 4;
            }
            if offset >= length {
                return false;
            }
            let n = payload[i + offset as usize] & 0x1F;
            if n == 7 {
                return true;
            } else if n >= 24 {
                // is this legal?
            }
            i += length as usize;
        }
        if i == payload.len() {
            return false;
        }
        return false;
    } else if nalu == 28 || nalu == 29 {
        // FU-A or FU-B
        if payload.len() < 2 {
            return false;
        }
        if (payload[1] & 0x80) == 0 {
            // not a starting fragment
            return false;
        }
        return payload[1] & 0x1F == 7;
    }
    return false;
}
