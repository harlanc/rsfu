use super::down_track::DownTrack;
use crate::buffer;
use crate::buffer::buffer::ExtPacket;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use webrtc::media::rtp::rtp_codec::{RTCRtpCodecParameters, RTPCodecType};
use webrtc::Error as WebrtcError;

// setVp8TemporalLayer is a helper to detect and modify accordingly the vp8 payload to reflect
// temporal changes in the SFU.
// VP8 temporal layers implemented according https://tools.ietf.org/html/rfc7741

fn set_vp8_temporal_layer(p: ExtPacket, d: &mut DownTrack) -> (Vec<u8>, u16, u8, bool) {
    let pkt = p.payload;
    let layer = d.temporal_layer.load(Ordering::Relaxed);

    let current_layer = layer as u16;
    let current_target_layer = (layer >> 16) as u16;

    if current_target_layer != current_layer {
        if pkt.tid <= current_target_layer as u8 {
            d.temporal_layer.store(
                ((current_target_layer as i32) << 16) | current_target_layer as i32,
                Ordering::Relaxed,
            )
        }
    } else if pkt.tid > current_layer as u8 {
        return (Vec::new(), 0, 0, true);
    }

    let mut rv_buf: Vec<u8> = Vec::new();

    let length = p.packet.payload.len();
    rv_buf.extend_from_slice(&d.payload[..length]);
    rv_buf.extend_from_slice(&p.packet.payload[..]);

    let pic_id = pkt.picture_id - d.simulcast.ref_pic_id + d.simulcast.p_ref_pic_id + 1;
    let tlz0_idx = pkt.tl0_picture_idx - d.simulcast.ref_tlz_idx + d.simulcast.p_ref_tlz_idx + 1;

    if p.head {
        d.simulcast.l_pic_id = pic_id;
        d.simulcast.l_tlz_idx = tlz0_idx;
    }

    modify_vp8_temporal_payload(
        &mut rv_buf,
        pkt.picture_id_idx as usize,
        pkt.tlz_idx as usize,
        pic_id,
        tlz0_idx,
        pkt.mbit,
    );

    (rv_buf, 0, 0, true)
}

fn modify_vp8_temporal_payload(
    payload: &mut [u8],
    pic_id_idx: usize,
    tlz0_idx: usize,
    pic_id: u16,
    tlz0_id: u8,
    m_bit: bool,
) {
    payload[pic_id_idx] = (pic_id >> 8) as u8;
    if m_bit {
        payload[pic_id_idx] = 0x80;
        payload[pic_id_idx + 1] = pic_id as u8;
    }

    payload[tlz0_idx] = tlz0_id;
}

// Do a fuzzy find for a codec in the list of codecs
// Used for lookup up a codec in an existing list to find a match
pub fn codec_parameters_fuzzy_search(
    needle: RTCRtpCodecParameters,
    hay_stack: &[RTCRtpCodecParameters],
) -> Result<RTCRtpCodecParameters> {
    // First attempt to match on MimeType + SDPFmtpLine
    for param in hay_stack {
        if (param.capability.mime_type == needle.capability.mime_type)
            && (param.capability.sdp_fmtp_line == needle.capability.sdp_fmtp_line)
        {
            return Ok(param.clone());
        }
    }

    // Fallback to just MimeType
    for param in hay_stack {
        if param.capability.mime_type == needle.capability.mime_type {
            return Ok(param.clone());
        }
    }

    Err(WebrtcError::ErrCodecNotFound.into())
}

fn ntp_to_millis_since_epoch(ntp: u64) -> u64 {
    // ntp time since epoch calculate fractional ntp as milliseconds
    // (lower 32 bits stored as 1/2^32 seconds) and add
    // ntp seconds (stored in higher 32 bits) as milliseconds
    (((ntp & 0xFFFFFFFF) * 1000) >> 32) + ((ntp >> 32) * 1000)
}

fn fast_forward_timestamp_amount(newest_timestamp: u32, reference_timestamp: u32) -> u32 {
    if buffer::buffer::is_timestamp_wrap_around(newest_timestamp, reference_timestamp) {
        return (newest_timestamp as u64 + 0x100000000 - reference_timestamp as u64) as u32;
    }

    if newest_timestamp < reference_timestamp {
        return 0;
    }

    newest_timestamp - reference_timestamp
}

struct NtpTime {
    ntp_time: u64,
}

impl NtpTime {
    fn duration(&self) -> Duration {
        let sec = self.ntp_time >> 32 * 1000000000;
        let frac = (self.ntp_time & 0xffffffff) * 1000000000;
        let mut nsec = (frac >> 32) as u32;
        if frac as u32 >= 0x80000000 {
            nsec += 1;
        }

        return Duration::new(sec, nsec);
    }

    fn time(&self) -> Option<SystemTime> {
        let now = SystemTime::now();

        now.checked_add(self.duration())
    }

    fn to_ntp_time(&self, t: SystemTime) -> NtpTime {
        let duration = t.duration_since(UNIX_EPOCH).unwrap();
        let mut nsec = duration.as_nanos() as u64;

        let sec = nsec / 1000000000;
        nsec = (nsec - sec * 1000000000) << 32;
        let mut frac = nsec / 1000000000;

        if nsec % 1000000000 >= 1000000000 / 2 {
            frac += 1;
        }

        NtpTime {
            ntp_time: sec << 32 | frac,
        }
    }
}
