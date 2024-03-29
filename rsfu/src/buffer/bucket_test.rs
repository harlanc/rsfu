#[cfg(test)]
mod tests {
    use crate::buffer::bucket::Bucket;
    use rtp::header::Header;
    use rtp::packet::Packet;
    use webrtc_util::{Marshal, Unmarshal};

    fn new_packet(seq_number: u16) -> Packet {
        Packet {
            header: Header {
                sequence_number: seq_number,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn init() -> Vec<Packet> {
        vec![
            new_packet(1),
            new_packet(3),
            new_packet(4),
            new_packet(6),
            new_packet(7),
            new_packet(10),
        ]
    }
    // #[allow(dead_code)]
    // fn marshal_to(buf: &mut [u8]) {
    //     // let aa = buf.remaining_mut();
    //     // let bb = aa;
    // }

    #[test]
    fn test_queue() {
        let packets = init();

        let mut bucket = Bucket::new(25000);
        let mut raw = vec![0u8; 25000];

        for p in &packets {
            let rv = p.marshal_to(&mut raw);
            assert!(rv.is_ok(), "marshal_to is OK");

            let length = rv.unwrap();
            let rv_add_packet = bucket.add_packet(&raw[..length], p.header.sequence_number, true);
            assert!(rv_add_packet.is_ok(), "add_packet is OK");
        }

        let bucket_data = bucket.get(6);
        assert!(bucket_data.is_some(), "bucket has data");
        let data = bucket_data.unwrap();
        let p = Packet::unmarshal(&mut &data[..]);

        assert!(p.is_ok(), "unmarshal is OK");
        assert_eq!(p.unwrap().header.sequence_number, 6);

        let packet_8 = new_packet(8);
        let rv = packet_8.marshal_to(&mut raw);
        assert!(rv.is_ok(), "marshal_to is OK");

        let length = rv.unwrap();
        let rv_add_packet =
            bucket.add_packet(&raw[..length], packet_8.header.sequence_number, false);
        assert!(rv_add_packet.is_ok(), "add_packet is OK");

        let bucket_data_8 = bucket.get(8);
        assert!(bucket_data_8.is_some(), "bucket has data");
        let data_8 = bucket_data_8.unwrap();
        let p8 = Packet::unmarshal(&mut &data_8[..]);

        assert!(p8.is_ok(), "unmarshal is OK");
        assert_eq!(p8.unwrap().header.sequence_number, 8);

        let rv_2 = bucket.add_packet(&raw[..length], 8, false);
        assert!(rv_2.is_err());
    }

    fn init2() -> Vec<Packet> {
        vec![new_packet(65533), new_packet(65534), new_packet(2)]
    }

    #[test]
    fn test_queue_edges() {
        let packets = init2();

        // let buf = vec![0u8; 25000];

        let mut bucket = Bucket::new(25000);
        let mut raw = vec![0u8; 25000];

        for p in &packets {
            let rv = p.marshal_to(&mut raw);
            assert!(rv.is_ok(), "marshal_to is OK");

            let length = rv.unwrap();
            let rv_add_packet = bucket.add_packet(&raw[..length], p.header.sequence_number, true);
            assert!(rv_add_packet.is_ok(), "add_packet is OK");
        }

        let bucket_data = bucket.get(65534);
        assert!(bucket_data.is_some(), "bucket has data");
        let data = bucket_data.unwrap();
        let p = Packet::unmarshal(&mut &data[..]);

        assert!(p.is_ok(), "unmarshal is OK");
        assert_eq!(p.unwrap().header.sequence_number, 65534);

        let packet_65535 = new_packet(65535);
        let rv = packet_65535.marshal_to(&mut raw);
        assert!(rv.is_ok(), "marshal_to is OK");

        let length = rv.unwrap();
        let rv_add_packet = bucket.add_packet(&raw[..length], 65535, false);
        assert!(rv_add_packet.is_ok(), "add_packet is OK");

        let bucket_data = bucket.get(65535);
        assert!(bucket_data.is_some(), "bucket has data");
        let data = bucket_data.unwrap();
        let p = Packet::unmarshal(&mut &data[..]);
        assert!(p.is_ok(), "unmarshal is OK");
        assert_eq!(p.unwrap().header.sequence_number, 65535);
    }

    use byteorder::{BigEndian, ByteOrder};

    #[test]
    fn test_bigend_bytes() {
        let xs: [u8; 2] = [4, 5];

        let sn = BigEndian::read_u16(&xs);

        println!("value:{}", sn);
        println!("value2:{}", 4 << 8 | 5);
    }
}
