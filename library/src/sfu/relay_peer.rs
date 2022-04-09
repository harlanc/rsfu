use super::router::Router;
use super::session::Session;
use crate::relay::relay::Peer;

use super::data_channel::DataChannel;
use super::publisher::PublisherTrack;
use super::sfu::WebRTCTransportConfig;
use tokio::sync::Mutex;

use std::sync::Arc;

pub struct RelayPeer {
    peer: Option<Peer>,
    session: Arc<dyn Session + Send + Sync>,
    // router: Arc<Box<dyn Router + Send + Sync>>,
    config: Arc<WebRTCTransportConfig>,
    tracks: Vec<PublisherTrack>,
    relay_peers: Vec<Peer>,
    data_channels: Vec<DataChannel>,
}

impl RelayPeer {
    pub fn new(
        peer: Peer,
        session: Arc<dyn Session + Send + Sync>,
        config: Arc<WebRTCTransportConfig>,
    ) -> Self {
        Self {
            peer: Some(peer),
            session,
            config,
            tracks: Vec::new(),
            relay_peers: Vec::new(),
            data_channels: Vec::new(),
        }
    }
}
