const PUBLISHER: u8 = 0;
const SUBSCRIBER: u8 = 1;

pub trait Peer {
    fn id(&self) -> String;
    //fn session()
}

struct PeerLocal {
    id: String,
}

impl Peer for PeerLocal {
    fn id(&self) -> String {
        self.id
    }
}
