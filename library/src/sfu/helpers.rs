use super::down_track::DownTrack;
use crate::buffer;
use crate::buffer::buffer::ExtPacket;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use webrtc::error::Result;

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType};
use webrtc::Error as WebrtcError;

use bytes::{Buf, BufMut, Bytes, BytesMut};

// setVp8TemporalLayer is a helper to detect and modify accordingly the vp8 payload to reflect
// temporal changes in the SFU.
// VP8 temporal layers implemented according https://tools.ietf.org/html/rfc7741

pub async fn set_vp8_temporal_layer(
    ext_packet: ExtPacket,
    d: &DownTrack,
) -> (Bytes, u16, u8, bool) {
    let pkt = ext_packet.payload;
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
        return (Bytes::default(), 0, 0, true);
    }

    let mut rv_buf = BytesMut::new();

    let length = ext_packet.packet.payload.len();
    rv_buf.copy_from_slice(&d.payload[..length]);
    rv_buf.copy_from_slice(&ext_packet.packet.payload[..]);

    let simulcast = &mut d.simulcast.lock().await;

    let pic_id = pkt.picture_id - simulcast.ref_pic_id + simulcast.p_ref_pic_id + 1;
    let tlz0_idx = pkt.tl0_picture_idx - simulcast.ref_tlz_idx + simulcast.p_ref_tlz_idx + 1;

    if ext_packet.head {
        simulcast.l_pic_id = pic_id;
        simulcast.l_tlz_idx = tlz0_idx;
    }

    modify_vp8_temporal_payload(
        &mut rv_buf,
        pkt.picture_id_idx as usize,
        pkt.tlz_idx as usize,
        pic_id,
        tlz0_idx,
        pkt.mbit,
    );

    (rv_buf.freeze(), 0, 0, true)
}

fn modify_vp8_temporal_payload(
    payload: &mut BytesMut,
    pic_id_idx: usize,
    tlz0_idx: usize,
    pic_id: u16,
    tlz0_id: u8,
    m_bit: bool,
) {
    // payload.get_mut(index)
    // let payload_slice = &mut payload[..];
    // let slice = mut payload.as_slice();

    payload[pic_id_idx] = (pic_id >> 8) as u8;
    if let Some(pic_id_ref) = payload.get_mut(pic_id_idx) {
        *pic_id_ref = (pic_id >> 8) as u8;
    }
    // payload_slice[pic_id_idx] = (pic_id >> 8) as u8;
    if m_bit {
        payload[pic_id_idx] |= 0x80;
        payload[pic_id_idx + 1] = pic_id as u8;

        // if let Some(ref0) = payload.get_mut(pic_id_idx) {
        //     *ref0 |= 0x80;
        // }

        // if let Some(ref1) = payload.get_mut(pic_id_idx + 1) {
        //     *ref1 = pic_id as u8;
        // }
    }

    payload[tlz0_idx] = tlz0_id;

    // if let Some(ref2) = payload.get_mut(tlz0_idx) {
    //     *ref2 = tlz0_id;
    // }
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

pub struct NtpTime {
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
}

impl From<NtpTime> for u64 {
    fn from(time: NtpTime) -> u64 {
        time.ntp_time
    }
}

pub fn to_ntp_time(t: SystemTime) -> NtpTime {
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
