use super::session::Session;
use std::sync::Arc;

const PUBLISHER: u8 = 0;
const SUBSCRIBER: u8 = 1;

pub trait Peer {
    fn id(&self) -> String;
    fn session(&self) -> Arc<dyn Session + Send + Sync>;
}

struct PeerLocal {
    id: String,
    session: Arc<dyn Session + Send + Sync>,
}

impl Peer for PeerLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn session(&self) -> Arc<dyn Session + Send + Sync> {
        self.session.clone()
    }
}
