#[cfg(test)]
mod tests {

    use crate::nack;
    use rtp::header::Header;
    use rtp::packet::Packet;

    use bytes::{Buf, Bytes, BytesMut};
    use rtcp::transport_feedbacks::transport_layer_nack::NackPair;

    use crate::bucket::Bucket;

    use webrtc_util::Marshal;

    fn new_packet(seq_number: u16) -> Packet {
        return Packet {
            header: Header {
                sequence_number: seq_number,
                ..Default::default()
            },
            ..Default::default()
        };
    }

    fn init() -> Vec<Packet> {
        let mut packets = Vec::new();

        packets.push(new_packet(1));
        packets.push(new_packet(3));
        packets.push(new_packet(4));
        packets.push(new_packet(6));
        packets.push(new_packet(7));
        packets.push(new_packet(10));

        return packets;
    }

    #[test]
    fn test_queue() {
        let packets = init();

        let buf = Vec::new();

        let bucket = Bucket::new(&buf);

        for p in &packets {
            let mut raw = BytesMut::new();
            print!("{}", p);
            let rv = p.marshal_to(&mut raw);

            assert!(rv.is_err(),"marshal_to is OK");
        }
    }
}
