#[cfg(test)]
mod tests {

    use rtcp::header;
    use rtcp::header::Header;
    use rtcp::header::PacketType;
    use rtcp::transport_feedbacks::transport_layer_cc::SymbolSizeTypeTcc;
    use rtcp::transport_feedbacks::transport_layer_cc::SymbolTypeTcc;
    use webrtc_util::Marshal;
    use crate::twcc::twcc::Responder;

    #[test]
    fn test_run_length_chunk() {
        #[derive(Default, Clone)]
        pub struct Fields {
            len: u16,
        }

        #[derive(Default, Clone)]
        pub struct Test {
            pub name: String,
            fields: Fields,
            args: Args,
            pub want_err: bool,
            want_bytes: Vec<u8>,
        }
        #[derive(Default, Clone)]
        pub struct Args {
            symbol: SymbolTypeTcc,
            run_length: u16,
        }

        let tests = [
            Test {
                name: String::from("Must not return error"),
                args: Args {
                    symbol: SymbolTypeTcc::PacketNotReceived,
                    run_length: 221,
                },
                want_err: false,
                want_bytes: vec![0, 0xdd],
                ..Default::default()
            },
            Test {
                name: String::from("Must set run length after padding"),
                fields: Fields { len: 1 },
                args: Args {
                    symbol: SymbolTypeTcc::PacketReceivedWithoutDelta,
                    run_length: 24,
                },
                want_bytes: vec![0, 0x60, 0x18],
                ..Default::default()
            },
        ];

        for test in tests {
            let mut t = Responder::new(0);
            t.len = test.fields.len;

            for _i in 0..test.fields.len {
                t.payload.push(0);
            }

            let rv = t.write_run_length_chunk(test.args.symbol as u16, test.args.run_length);

            assert!(rv.is_ok());

            assert_eq!(test.want_bytes, t.payload);
        }
    }
    #[test]
    fn test_write_status_symbol_chunk() {
        #[derive(Default, Clone)]
        pub struct Fields {
            len: u16,
        }

        #[derive(Default, Clone)]
        pub struct Test {
            pub name: String,
            fields: Fields,
            args: Args,
            want_bytes: Vec<u8>,
        }
        #[derive(Default, Clone)]
        pub struct Args {
            symbol_size: SymbolSizeTypeTcc,
            symbols: Vec<SymbolTypeTcc>,
        }
        let tests = [
            Test {
                name: String::from("Must not return error"),
                args: Args {
                    symbol_size: SymbolSizeTypeTcc::OneBit,
                    symbols: vec![
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketNotReceived,
                    ],
                },

                want_bytes: vec![0x9F, 0x1C],
                ..Default::default()
            },
            Test {
                name: String::from("Must set symbol chunk after padding"),
                fields: Fields { len: 1 },
                args: Args {
                    symbol_size: SymbolSizeTypeTcc::TwoBit,
                    symbols: vec![
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketReceivedWithoutDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketReceivedSmallDelta,
                        SymbolTypeTcc::PacketNotReceived,
                        SymbolTypeTcc::PacketNotReceived,
                    ],
                },

                want_bytes: vec![0x0, 0xcd, 0x50],
                ..Default::default()
            },
        ];

        for test in tests {
            let mut t = Responder::new(0);
            t.len = test.fields.len;

            for _i in 0..test.fields.len {
                t.payload.push(0);
            }

            let mut idx = 0;

            for symbol in test.args.symbols {
                t.create_status_symbol_chunk(test.args.symbol_size, symbol as u16, idx);

                idx += 1;
            }

            let rv = t.write_status_symbol_chunk(test.args.symbol_size);

            assert!(rv.is_ok());
            assert_eq!(test.want_bytes, t.payload);
        }
    }

    #[test]
    fn test_write_delta() {
        let a = 0x8000;

        #[derive(Default, Clone)]
        pub struct Fields {
            delta_len: usize,
        }

        #[derive(Default, Clone)]
        pub struct Args {
            delta_type: SymbolTypeTcc,
            delta: u16,
        }
        #[derive(Default, Clone)]
        pub struct Test {
            pub name: String,
            fields: Fields,
            args: Args,
            want_bytes: Vec<u8>,
        }

        let tests = [
            Test {
                name: String::from("Must set correct small delta"),
                args: Args {
                    delta_type: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 255,
                },

                want_bytes: vec![0xFF],
                ..Default::default()
            },
            Test {
                name: String::from("Must set correct small delta with padding"),

                fields: Fields { delta_len: 1 },
                args: Args {
                    delta_type: SymbolTypeTcc::PacketReceivedSmallDelta,
                    delta: 255,
                },

                want_bytes: vec![0, 0xFF],
                ..Default::default()
            },
            Test {
                name: String::from("Must set correct large delta"),
                args: Args {
                    delta_type: SymbolTypeTcc::PacketReceivedLargeDelta,
                    delta: 32767,
                },

                want_bytes: vec![0x7F, 0xFF],
                ..Default::default()
            },
            Test {
                name: String::from("Must set correct large delta with padding"),
                fields: Fields { delta_len: 1 },
                args: Args {
                    delta_type: SymbolTypeTcc::PacketReceivedLargeDelta,
                    delta: a,
                },

                want_bytes: vec![0, 0x80, 0x00],
                ..Default::default()
            },
        ];

        for test in tests {
            let mut t = Responder::new(0);
            t.delta_len = test.fields.delta_len;

            for _i in 0..test.fields.delta_len {
                t.deltas.push(0);
            }

            let rv = t.write_delta(test.args.delta_type, test.args.delta);
            assert!(rv.is_ok());

            assert_eq!(test.want_bytes, t.deltas[..t.delta_len]);
        }
    }

    #[test]
    fn test_write_header() {
        #[derive(Default, Clone)]
        pub struct Fields {
            tcc_pkt_ctn: u8,
            s_ssrc: u32,
            m_ssrc: u32,
        }

        #[derive(Default, Clone)]
        pub struct Args {
            b_sn: u16,
            packet_count: u16,
            ref_time: u32,
        }
        #[derive(Default, Clone)]
        pub struct Test {
            pub name: String,
            fields: Fields,
            args: Args,
            want_bytes: Vec<u8>,
        }

        let tests = [Test {
            name: String::from("Must construct correct header"),
            fields: Fields {
                tcc_pkt_ctn: 23,
                s_ssrc: 4195875351,
                m_ssrc: 1124282272,
            },
            args: Args {
                b_sn: 153,
                packet_count: 1,
                ref_time: 4057090,
            },

            want_bytes: vec![
                0xfa, 0x17, 0xfa, 0x17, 0x43, 0x3, 0x2f, 0xa0, 0x0, 0x99, 0x0, 0x1, 0x3d, 0xe8,
                0x2, 0x17,
            ],
        }];

        for test in tests {
            let mut t = Responder::new(0);
            t.pkt_ctn = test.fields.tcc_pkt_ctn;
            t.m_ssrc = test.fields.m_ssrc;
            t.s_ssrc = test.fields.s_ssrc;

            let rv = t.write_header(test.args.b_sn, test.args.packet_count, test.args.ref_time);

            assert!(rv.is_ok());

            assert_eq!(test.want_bytes, t.payload[..16]);
        }
    }

    #[test]
    fn test_tcc_packet() {
        let want_bytes = vec![
            0xfa, 0x17, 0xfa, 0x17, 0x43, 0x3, 0x2f, 0xa0, 0x0, 0x99, 0x0, 0x1, 0x3d, 0xe8, 0x2,
            0x17, 0x60, 0x18, 0x0, 0xdd, 0x9F, 0x1C, 0xcd, 0x50,
        ];
        let delta = vec![0xff, 0x80, 0xaa];
        let symbol1 = vec![
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketNotReceived,
        ];

        let symbol2 = vec![
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketReceivedWithoutDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketReceivedSmallDelta,
            SymbolTypeTcc::PacketNotReceived,
            SymbolTypeTcc::PacketNotReceived,
        ];

        let mut t = Responder::new(0);
        t.pkt_ctn = 23;
        t.m_ssrc = 1124282272;
        t.s_ssrc = 4195875351;

        let mut rv = t.write_header(153, 1, 4057090);
        assert!(rv.is_ok());
        rv = t.write_run_length_chunk(SymbolTypeTcc::PacketReceivedWithoutDelta as u16, 24);
        assert!(rv.is_ok());
        rv = t.write_run_length_chunk(SymbolTypeTcc::PacketNotReceived as u16, 221);
        assert!(rv.is_ok());

        for (i, v) in symbol1.iter().enumerate() {
            t.create_status_symbol_chunk(SymbolSizeTypeTcc::OneBit, v.clone() as u16, i as u16);
        }
        rv = t.write_status_symbol_chunk(SymbolSizeTypeTcc::OneBit);
        assert!(rv.is_ok());

        for (i, v) in symbol2.iter().enumerate() {
            t.create_status_symbol_chunk(SymbolSizeTypeTcc::TwoBit, v.clone() as u16, i as u16);
        }

        rv = t.write_status_symbol_chunk(SymbolSizeTypeTcc::TwoBit);
        assert!(rv.is_ok());

        t.delta_len = delta.len();
        assert_eq!(want_bytes, t.payload[..24]);

        let mut p_len = t.len + t.delta_len as u16 + 4;
        let pad = (p_len % 4) != 0;

        while p_len % 4 != 0 {
            p_len += 1;
        }

        let hdr = Header {
            padding: pad,
            length: p_len / 4 - 1,
            count: header::FORMAT_TCC,
            packet_type: PacketType::TransportSpecificFeedback,
        };

        assert_eq!(p_len as usize, want_bytes.len() + 3 + 4 + 1);

        let mut raw_packet_data: Vec<u8> = Vec::new();
        let rv = hdr.marshal_to(&mut raw_packet_data[..]); //header
        assert!(rv.is_ok());

        //assert_eq!(raw_packet_data,)
    }
}
