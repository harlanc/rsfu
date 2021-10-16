use rtp::packet::Packet;

struct PendingPackets {
    arrival_time: u64,
    packet: Vec<u8>,
}

struct ExtPacket {
    head: bool,
    cycle: u32,
    arrival: i64,
    packet: Packet,
    key_frame: bool,
}
