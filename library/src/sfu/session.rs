use super::audio_observer::AudioObserver;
use super::peer::Peer;
use super::receiver::Receiver;
use super::relay_peer::RelayPeer;
use super::router::Router;
use super::sfu::WebRTCTransportConfig;
use crate::relay::relay;
use anyhow::Result;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use std::sync::Arc;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

const AUDIO_LEVELS_METHOD: &'static str = "audioLevels";
#[async_trait]
pub trait Session {
    fn id(&self) -> String;
    fn publish(
        &mut self,
        router: Arc<dyn Router + Send + Sync>,
        r: Arc<dyn Receiver + Send + Sync>,
    );
    fn subscribe(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    fn add_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    fn get_peer(&self, peer_id: String) -> Option<Arc<dyn Peer + Send + Sync>>;
    fn remove_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    async fn add_relay_peer(&mut self, peer_id: String, signal_data: String) -> Result<String>;
    fn audio_obserber(&self) -> Option<AudioObserver>;

    fn add_data_channel(&mut self, owner: String, dc: RTCDataChannel);
    fn get_data_channel_middlewares(&self) -> Vec<Option<RTCDataChannel>>;
    fn get_fanout_data_channel_labels(&self) -> Vec<String>;
    fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Option<RTCDataChannel>>;
    fn fanout_message(&self, origin: String, label: String, msg: DataChannelMessage);
    fn peers(&self) -> Vec<Arc<dyn Peer + Send + Sync>>;
    fn relay_peers(&self) -> Vec<Option<RelayPeer>>;

    // fn subscribe()
}

pub struct SessionLocal {
    id: String,
    config: WebRTCTransportConfig,
    peers: HashMap<String, Arc<dyn Peer + Send + Sync>>,
    relay_peers: Arc<HashMap<String, Option<RelayPeer>>>,
    closed: AtomicBool,
    audio_observer: Option<AudioObserver>,
    fanout_data_channels: Vec<String>,
    data_channels: Vec<Option<RTCDataChannel>>,
    on_close_handler: fn(),
}

impl SessionLocal {
    //  pub fn new(
    //      id: String,
    //      dcs: Vec<Option<RTCDataChannel>>,
    //      cfg: WebRTCTransportConfig,
    //  ) -> Box<dyn Session> {
    //  }

    fn id(&self) -> String {
        self.id
    }

    fn audio_obserber(&self) -> Option<AudioObserver> {
        self.audio_observer
    }

    fn get_data_channel_middlewares(&self) -> Vec<Option<RTCDataChannel>> {
        self.data_channels
    }

    fn get_fanout_data_channel_labels(&self) -> Vec<String> {
        self.fanout_data_channels
    }

    fn add_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>) {
        self.peers.insert(peer.id(), peer);
    }

    fn get_peer(&self, peer_id: String) -> Option<Arc<dyn Peer + Send + Sync>> {
        let rv = self.peers.get(&peer_id).unwrap().clone();
        Some(rv)
    }

    fn remove_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>) {}

    async fn add_relay_peer(&mut self, peer_id: String, signal_data: String) -> Result<String> {
        let meta = relay::PeerMeta {
            peer_id,
            session_id: self.id.clone(),
        };

        let conf = relay::PeerConfig {
            setting_engine: self.config.setting.clone(),
            ice_servers: self.config.configuration.ice_servers.clone(),
        };

        let mut relay_peer = relay::Peer::new(meta, conf)?;

        relay_peer.answer(signal_data).await?;

        let relay_peers = Arc::clone(&self.relay_peers);

        //   relay_peer.on_ready(Box::new(move || {
        //       let relay_peers_in = Arc::clone(&relay_peers);

        //       Box::pin(async move {

        //          let rp =

        //          relay_peers_in.insert(peer_id, )

        //       })
        //   }))

        Ok(String::from(""))
    }

    fn add_data_channel(&mut self, owner: String, dc: RTCDataChannel) {}

    fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Option<RTCDataChannel>> {
        Vec::new()
    }
    fn fanout_message(&self, origin: String, label: String, msg: DataChannelMessage) {}
    fn peers(&self) -> Vec<Arc<dyn Peer + Send + Sync>> {
        Vec::new()
    }
    fn relay_peers(&self) -> Vec<Option<RelayPeer>> {
        Vec::new()
    }

    fn publish(
        &mut self,
        router: Arc<dyn Router + Send + Sync>,
        r: Arc<dyn Receiver + Send + Sync>,
    ) {
    }
    fn subscribe(&mut self, peer: Arc<dyn Peer + Send + Sync>) {}
}
