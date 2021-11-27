use std::borrow::BorrowMut;
use std::time::{SystemTime, UNIX_EPOCH};

use std::sync::Arc;

use std::collections::HashMap;

use tokio::sync::Mutex;

const IGNORE_RETRANSMISSION: u8 = 100;
#[derive(Default, Clone)]
struct PacketMeta {
    // Original sequence number from stream.
    // The original sequence number is used to find the original
    // packet from publisher
    source_seq_no: u16,
    // Modified sequence number after offset.
    // This sequence number is used for the associated
    // down track, is modified according the offsets, and
    // must not be shared
    target_seq_no: u16,
    // Modified timestamp for current associated
    // down track.
    timestamp: u32,
    // The last time this packet was nack requested.
    // Sometimes clients request the same packet more than once, so keep
    // track of the requested packets helps to avoid writing multiple times
    // the same packet.
    // The resolution is 1 ms counting after the sequencer start time.
    last_nack: u32,
    // Spatial layer of packet
    layer: u8,
    // Information that differs depending the codec
    misc: u32,
}

impl PacketMeta {
    fn set_vp8_payload_meta(&mut self, tlz0_idx: u8, pic_id: u16) {
        self.misc = ((tlz0_idx as u32) << 16) | (pic_id as u32);
    }
    fn get_vp8_payload_meta(&self) -> (u8, u16) {
        ((self.misc >> 16) as u8, self.misc as u16)
    }
}
#[derive(Default)]
struct Sequencer {
    init: bool,
    max: i32,
    seq: HashMap<i32, PacketMeta>,
    step: i32,
    head_sn: u16,
    start_time: u64,
}

#[derive(Default)]
pub struct AtomicSequencer {
    sequencer: Arc<Mutex<Sequencer>>,
}

impl Sequencer {
    pub fn new(max_track: i32) -> Self {
        Self {
            max: max_track,
            seq: HashMap::new(),
            start_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            ..Default::default()
        }
    }

    // fn push(&self,sn :u16)
}

impl AtomicSequencer {
    pub fn new(max_track: i32) -> Self {
        Self {
            sequencer: Arc::new(Mutex::new(Sequencer::new(max_track))),
        }
    }

    async fn push(
        &mut self,
        sn: u16,
        off_sn: u16,
        timastamp: u32,
        layer: u8,
        head: bool,
    ) -> Option<PacketMeta> {
        let mut sequencer = self.sequencer.lock().await;

        if !sequencer.init {
            sequencer.head_sn = off_sn;
            sequencer.init = true;
        }

        let mut step = 0;
        if head {
            let inc = off_sn - sequencer.head_sn;

            for i in 1..inc {
                sequencer.step += 1;
                if sequencer.step >= sequencer.max {
                    sequencer.step = 0;
                }
            }
            step = sequencer.step;
            sequencer.head_sn = off_sn;
        } else {
            step = sequencer.step - (sequencer.head_sn - off_sn) as i32;
            if step < 0 {
                if step * -1 >= sequencer.max {
                    return None;
                }

                step = step + sequencer.max;
            }
        }

        let cur_step = sequencer.step;

        sequencer.seq.insert(
            cur_step,
            PacketMeta {
                source_seq_no: sn,
                target_seq_no: off_sn,
                timestamp: timastamp,
                layer: layer,
                ..Default::default()
            },
        );

        sequencer.step += 1;

        if sequencer.step >= sequencer.max {
            sequencer.step = 0;
        }

        Some(sequencer.seq.get(&sequencer.step).unwrap().clone())
    }
}
