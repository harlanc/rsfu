use rtcp::transport_feedbacks::transport_layer_nack::NackPair;

pub const MAX_NACK_TIMES: u8 = 3;
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
    pub fn new() -> Self {
        Self {
            nacks: Vec::new(),
            key_frame_seq_number: 0,
        }
    }
    pub fn push(&mut self, sn: u32) {
        /*find the specified nack ele according to the sequence number*/
        let rv = self
            .nacks
            .binary_search_by_key(&sn, |&Nack { seq_number, nackd }| seq_number);

        let insert_index: usize;
        match rv {
            Ok(_) => {
                /*exists then return*/
                return;
            }
            Err(index) => {
                /*not exists then insert the new nack*/
                insert_index = index;
            }
        }

        let nack = Nack::new(sn, 0);
        self.nacks.insert(insert_index, nack);
    }

    pub fn remove(&mut self, sn: u32) -> Option<Nack> {
        if let Ok(index) = self
            .nacks
            .binary_search_by_key(&sn, |&Nack { seq_number, nackd }| seq_number)
        {
            return Some(self.nacks.remove(index));
        }

        None
    }

    pub fn pairs(&mut self, head_seq_number: u32) -> (Option<Vec<NackPair>>, bool) {
        if self.nacks.len() == 0 {
            return (None, false);
        }

        let mut ask_key_frame: bool = false;

        for v in &self.nacks {
            if v.nackd >= MAX_NACK_TIMES {
                if v.seq_number > self.key_frame_seq_number {
                    self.key_frame_seq_number = v.seq_number;
                    ask_key_frame = true;
                }
                continue;
            }

            if v.seq_number >= head_seq_number - 2 {}
        }

        (None, false)
    }
}

#[cfg(test)]
mod tests {

    use super::NackQueue;

    #[test]
    fn test_nack_queue_push() {
        let mut nack_queue = NackQueue::new();
        nack_queue.push(3);
        nack_queue.push(4);
        nack_queue.push(2);
        nack_queue.push(36);

        assert_eq!(2, nack_queue.nacks[0].seq_number);
        assert_eq!(3, nack_queue.nacks[1].seq_number);
        assert_eq!(4, nack_queue.nacks[2].seq_number);
        assert_eq!(36, nack_queue.nacks[3].seq_number);
    }

    #[test]
    fn test_nack_queue_remove() {
        let mut nack_queue = NackQueue::new();
        nack_queue.push(3);
        nack_queue.push(4);
        nack_queue.push(2);
        nack_queue.push(36);

        nack_queue.remove(4);

        assert_eq!(2, nack_queue.nacks[0].seq_number);
        assert_eq!(3, nack_queue.nacks[1].seq_number);
        assert_eq!(36, nack_queue.nacks[2].seq_number);

        nack_queue.remove(3);

        assert_eq!(2, nack_queue.nacks[0].seq_number);
        assert_eq!(36, nack_queue.nacks[1].seq_number);
    }
}
