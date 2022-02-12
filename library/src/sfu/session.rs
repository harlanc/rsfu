use super::audio_observer::AudioObserver;
use super::peer::Peer;
use super::receiver::Receiver;
use super::relay_peer::RelayPeer;
use super::router::Router;
use anyhow::Result;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::sync::Arc;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
pub trait Session {
    fn id() -> String;
    fn publish(router: Arc<dyn Router + Send + Sync>, r: Arc<dyn Receiver + Send + Sync>);
    fn subscribe(peer: Arc<dyn Peer + Send + Sync>);
    fn add_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    fn get_peer(&self) -> Arc<dyn Peer + Send + Sync>;
    fn remove_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    fn add_relay_peer(&mut self, peer_id: String, signal_data: BytesMut) -> Result<BytesMut>;
    fn audio_obserber(&self) -> Option<AudioObserver>;

    fn add_data_channel(owner: String, dc: RTCDataChannel);
    fn get_data_channel_middlewares(&self) -> Vec<Option<RTCDataChannel>>;
    fn get_fanout_data_channel_labels(&self) -> Vec<String>;
    fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Option<RTCDataChannel>>;
    fn fanout_message(origin: String, label: String, msg: DataChannelMessage);
    fn peers(&self) -> Vec<Arc<dyn Peer + Send + Sync>>;
    fn relay_peers(&self) -> Vec<Option<RelayPeer>>;

    // fn subscribe()
}
