pub struct Nack {
    seq_number: u32,
    nackd: u8,
}

impl Nack {
    pub fn new(seq_number: u32, nackd: u8) -> Self {
        Self { seq_number, nackd }
    }
}

use rtcp::receiver_report::ReceiverReport;
pub struct NackQueue {
    nacks: Vec<Nack>,
    key_frame_seq_number: u32,
}

impl NackQueue {
    pub fn push(&mut self, sn: u32) {
        let rv = self
            .nacks
            .binary_search_by_key(&sn, |&Nack { seq_number, nackd }| seq_number);

        let insert_index: usize;
        match rv {
            Ok(_) => {
                return;
            }
            Err(index) => {
                insert_index = index;
            }
        }
        let nack = Nack::new(sn, 0);

        self.nacks.insert(insert_index, nack);
    }

    pub fn remove(extSN: u32) {}
}
