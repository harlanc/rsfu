use super::down_track::DownTrack;
use crate::buffer::buffer::ExtPacket;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

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
