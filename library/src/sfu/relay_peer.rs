use super::router::Router;
use super::session::Session;
use crate::relay::relay::Peer;

use super::publisher::PublisherTrack;
use super::sfu::WebRTCTransportConfig;
use super::data_channel::DataChannel;

use std::sync::Arc;

pub struct RelayPeer {
    peer: Option<Peer>,
    session: Arc<Box<dyn Session + Send + Sync>>,
    router: Arc<Box<dyn Router + Send + Sync>>,
    config: Option<WebRTCTransportConfig>,
    tracks: Vec<PublisherTrack>,
    relay_peers: Vec<Peer>,
    data_channels: Vec<DataChannel>,
}
