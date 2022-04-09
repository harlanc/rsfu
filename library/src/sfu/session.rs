use super::audio_observer::AudioObserver;
use super::data_channel::DataChannel;
use super::peer::Peer;
use super::receiver::Receiver;
use super::relay_peer::RelayPeer;
use super::router::Router;
use super::sfu::WebRTCTransportConfig;
use crate::relay::relay;
use anyhow::Result;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;

use tokio::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

const AUDIO_LEVELS_METHOD: &'static str = "audioLevels";

pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;
#[async_trait]
pub trait Session {
    fn id(&self) -> String;
    fn publish(
        &mut self,
        router: Arc<Mutex<dyn Router + Send + Sync>>,
        r: Arc<Mutex<dyn Receiver + Send + Sync>>,
    );
    fn subscribe(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    fn add_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    fn get_peer(&self, peer_id: String) -> Option<Arc<dyn Peer + Send + Sync>>;
    async fn remove_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>);
    async fn add_relay_peer(
        self: Arc<Self>,
        peer_id: String,
        signal_data: String,
    ) -> Result<String>;
    fn audio_obserber(&mut self) -> Option<&mut AudioObserver>;

    fn add_data_channel(self: Arc<Self>, owner: String, dc: Arc<RTCDataChannel>);
    fn get_data_channel_middlewares(&self) -> Vec<Arc<DataChannel>>;
    fn get_fanout_data_channel_labels(&self) -> Vec<String>;
    fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Arc<RTCDataChannel>>;
    fn fanout_message(&self, origin: String, label: String, msg: DataChannelMessage);
    fn peers(&self) -> Vec<Arc<dyn Peer + Send + Sync>>;
    fn relay_peers(&self) -> Vec<Option<RelayPeer>>;

    // fn subscribe()
}

pub struct SessionLocal {
    id: String,
    config: Arc<WebRTCTransportConfig>,
    peers: HashMap<String, Arc<dyn Peer + Send + Sync>>,
    relay_peers: Arc<Mutex<HashMap<String, Arc<RelayPeer>>>>,
    closed: AtomicBool,
    audio_observer: Option<AudioObserver>,
    fanout_data_channels: Vec<String>,
    data_channels: Vec<Arc<DataChannel>>,
    on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,
}

impl SessionLocal {
    pub async fn new(
        id: String,
        dcs: Vec<Option<DataChannel>>,
        config: Arc<WebRTCTransportConfig>,
    ) -> Arc<Mutex<dyn Session + Send + Sync>> {
        let s = SessionLocal {
            id,
            config,
            peers: HashMap::new(),
            relay_peers: Arc::new(Mutex::new(HashMap::new())),
            closed: AtomicBool::new(false),
            audio_observer: None,
            fanout_data_channels: Vec::new(),
            data_channels: Vec::new(),
            on_close_handler: Arc::new(Mutex::new(None)),
        };

        return Arc::new(Mutex::new(s));
    }

    async fn get_relay_peer(&self, peer_id: String) -> Option<Arc<RelayPeer>> {
        let relay_peers = self.relay_peers.lock().await;
        if let Some(relay_peer) = relay_peers.get(&peer_id) {
            Some(relay_peer.clone())
        } else {
            None
        }
    }

    async fn close(&mut self) {
        self.closed.store(true, Ordering::Relaxed);

        let mut close_handler = self.on_close_handler.lock().await;

        if let Some(f) = &mut *close_handler {
            f().await;
        }
    }
}
#[async_trait]
impl Session for SessionLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn audio_obserber(&mut self) -> Option<&mut AudioObserver> {
        self.audio_observer.as_mut()
    }

    fn get_data_channel_middlewares(&self) -> Vec<Arc<DataChannel>> {
        self.data_channels.clone()
    }

    fn get_fanout_data_channel_labels(&self) -> Vec<String> {
        self.fanout_data_channels.clone()
    }

    fn add_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>) {
        self.peers.insert(peer.id(), peer);
    }

    fn get_peer(&self, peer_id: String) -> Option<Arc<dyn Peer + Send + Sync>> {
        let rv = self.peers.get(&peer_id).unwrap().clone();
        Some(rv)
    }

    async fn remove_peer(&mut self, peer: Arc<dyn Peer + Send + Sync>) {
        let id = peer.id();

        if let Some(p) = self.peers.get(&id) {
            self.peers.remove(&id);
        }

        let peer_count = self.peers.len() + self.relay_peers.lock().await.len();

        if peer_count == 0 {
            self.close().await;
        }
    }

    async fn add_relay_peer(
        self: Arc<Self>,
        peer_id: String,
        signal_data: String,
    ) -> Result<String> {
        let peer_id_clone = peer_id.clone();
        let meta = relay::PeerMeta {
            peer_id: peer_id_clone,
            session_id: self.id.clone(),
        };

        let conf = relay::PeerConfig {
            setting_engine: self.config.setting.clone(),
            ice_servers: self.config.configuration.ice_servers.clone(),
        };

        let mut peer = relay::Peer::new(meta, conf)?;
        let resp = peer.answer(signal_data).await?;

        let relay_peers = Arc::clone(&self.relay_peers);
        let relay_peers_1 = Arc::clone(&self.relay_peers);

        let peer_id_out = peer_id.clone();
        let peer_id_out_1 = peer_id.clone();
        let peer_out = peer.clone();

        peer.on_ready(Box::new(move || {
            let relay_peers_in = Arc::clone(&relay_peers);
            let peer_id_in = peer_id_out.clone();
            let relay_peer = RelayPeer::new(peer_out.clone(), self.clone(), self.config.clone());

            Box::pin(async move {
                let mut val = relay_peers_in.lock().await;
                val.insert(peer_id_in, Arc::new(relay_peer));
            })
        }))
        .await;

        peer.on_close(Box::new(move || {
            let relay_peers_in = Arc::clone(&relay_peers_1);
            let peer_id_in = peer_id_out_1.clone();

            Box::pin(async move {
                let mut val = relay_peers_in.lock().await;
                val.remove(&peer_id_in);
            })
        }))
        .await;

        Ok(resp)
    }

    fn add_data_channel(self: Arc<Self>, owner: String, dc: Arc<RTCDataChannel>) {
        let label = dc.label();

        let s = self.clone();
        let owner_out = owner.clone();

        // for lab in &self.fanout_data_channels {
        //     if label == lab {
        //         dc.on_message(Box::new(move |msg: DataChannelMessage| {
        //             let owner_in = owner_out.clone();
        //             Box::pin(async move {
        //                 s.fanout_message(owner_in, label.to_string(), msg);
        //             })
        //         }));
        //         return;
        //     }
        // }
    }

    fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Arc<RTCDataChannel>> {
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
        router: Arc<Mutex<dyn Router + Send + Sync>>,
        r: Arc<Mutex<dyn Receiver + Send + Sync>>,
    ) {
    }
    fn subscribe(&mut self, peer: Arc<dyn Peer + Send + Sync>) {}
}
