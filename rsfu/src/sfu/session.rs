use super::audio_observer::AudioObserver;
use super::data_channel::DataChannel;
use super::peer::ChannelAPIMessage;
use super::peer::Peer;
use super::receiver::Receiver;
use super::relay_peer::RelayPeer;
use super::router::Router;
use super::sfu::WebRTCTransportConfig;
use crate::relay::relay;
use anyhow::Result;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::data_channel_state::RTCDataChannelState;
use webrtc::data_channel::RTCDataChannel;

use std::str;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use super::subscriber::API_CHANNEL_LABEL;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};

const AUDIO_LEVELS_METHOD: &'static str = "audioLevels";

pub type OnCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;
#[async_trait]
pub trait Session {
    fn id(&self) -> String;
    async fn publish(
        &self,
        router: Arc<dyn Router + Send + Sync>,
        r: Arc<dyn Receiver + Send + Sync>,
    );
    async fn subscribe(&self, peer: Arc<dyn Peer + Send + Sync>);
    async fn add_peer(&self, peer: Arc<dyn Peer + Send + Sync>);
    async fn get_peer(&self, peer_id: String) -> Option<Arc<dyn Peer + Send + Sync>>;
    async fn remove_peer(&self, peer: Arc<dyn Peer + Send + Sync>);
    async fn add_relay_peer(
        self: Arc<Self>,
        peer_id: String,
        signal_data: String,
    ) -> Result<String>;
    fn audio_obserber(&self) -> Option<Arc<Mutex<AudioObserver>>>;

    async fn add_data_channel(&self, owner: String, dc: Arc<RTCDataChannel>);
    fn get_data_channel_middlewares(&self) -> Arc<Mutex<Vec<Arc<DataChannel>>>>;
    async fn get_fanout_data_channel_labels(&self) -> Vec<String>;
    async fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Arc<RTCDataChannel>>;
    async fn fanout_message(&self, origin: String, label: String, msg: DataChannelMessage);
    async fn peers(&self) -> Vec<Arc<dyn Peer + Send + Sync>>;
    async fn relay_peers(&self) -> Vec<Arc<RelayPeer>>;
    async fn on_close(&self, f: OnCloseFn);

    // fn subscribe()
}

pub struct SessionLocal {
    id: String,
    config: Arc<WebRTCTransportConfig>,
    peers: Arc<Mutex<HashMap<String, Arc<dyn Peer + Send + Sync>>>>,
    relay_peers: Arc<Mutex<HashMap<String, Arc<RelayPeer>>>>,
    closed: AtomicBool,
    audio_observer: Option<Arc<Mutex<AudioObserver>>>,
    fanout_data_channels: Arc<Mutex<Vec<String>>>,
    data_channels: Arc<Mutex<Vec<Arc<DataChannel>>>>,
    on_close_handler: Arc<Mutex<Option<OnCloseFn>>>,
}

impl SessionLocal {
    pub async fn new(
        id: String,
        dcs: Arc<Mutex<Vec<Arc<DataChannel>>>>,
        config: Arc<WebRTCTransportConfig>,
    ) -> Arc<dyn Session + Send + Sync> {
        let s = SessionLocal {
            id,
            config: config.clone(),
            peers: Arc::new(Mutex::new(HashMap::new())),
            relay_peers: Arc::new(Mutex::new(HashMap::new())),
            closed: AtomicBool::new(false),
            audio_observer: None,
            fanout_data_channels: Arc::new(Mutex::new(Vec::new())),
            data_channels: dcs,
            on_close_handler: Arc::new(Mutex::new(None)),
        };

        let session_local = Arc::new(s);
        let session_local_in = session_local.clone();

        tokio::spawn(async move {
            if let Err(err) = session_local_in
                .audio_level_observer(config.router.audio_level_interval)
                .await
            {
                log::error!("session_local err:{}", err);
            }
        });

        session_local
    }
    #[allow(dead_code)]
    async fn get_relay_peer(&self, peer_id: String) -> Option<Arc<RelayPeer>> {
        let relay_peers = self.relay_peers.lock().await;
        if let Some(relay_peer) = relay_peers.get(&peer_id) {
            Some(relay_peer.clone())
        } else {
            None
        }
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);

        let mut close_handler = self.on_close_handler.lock().await;
        if let Some(f) = &mut *close_handler {
            f().await;
        }
    }

    async fn fanout_message(data_channels: Vec<Arc<RTCDataChannel>>, msg: DataChannelMessage) {
        // let data_channels = self.get_data_channels(origin, label).await;

        for dc in data_channels {
            let s = match str::from_utf8(msg.data.as_ref()) {
                Ok(v) => v,
                Err(e) => panic!("Invalid UTF-8 sequence: {}", e),
            };
            if let Err(err) = if msg.is_string {
                dc.send_text(s.to_string()).await
            } else {
                dc.send(&msg.data).await
            } {
                log::error!("fanout_message send error:{}", err);
            }
        }
    }

    async fn audio_level_observer(&self, audio_level_interval: i32) -> Result<()> {
        let mut audio_level_interval_new: u64 = audio_level_interval as u64;
        if audio_level_interval_new == 0 {
            audio_level_interval_new = 1000
        }

        loop {
            sleep(Duration::from_millis(audio_level_interval_new)).await;

            if self.closed.load(Ordering::Relaxed) {
                return Ok(());
            }

            if let Some(audio_observer) = &self.audio_observer {
                match audio_observer.lock().await.calc().await {
                    Some(levels) => {
                        let msg = ChannelAPIMessage {
                            method: String::from(AUDIO_LEVELS_METHOD),
                            params: levels,
                        };
                        let msg_str = serde_json::to_string(&msg)?;

                        let dcs = self
                            .get_data_channels(String::from(""), String::from(API_CHANNEL_LABEL))
                            .await;

                        for dc in dcs {
                            dc.send_text(msg_str.clone()).await?;
                        }
                    }
                    None => {
                        continue;
                    }
                }
            }
        }
    }
}
#[async_trait]
impl Session for SessionLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn audio_obserber(&self) -> Option<Arc<Mutex<AudioObserver>>> {
        self.audio_observer.clone()
    }

    fn get_data_channel_middlewares(&self) -> Arc<Mutex<Vec<Arc<DataChannel>>>> {
        self.data_channels.clone()
    }

    async fn get_fanout_data_channel_labels(&self) -> Vec<String> {
        self.fanout_data_channels.lock().await.clone()
    }

    async fn add_peer(&self, peer: Arc<dyn Peer + Send + Sync>) {
        log::info!("add_peer...");
        let id = peer.id().await;
        self.peers.lock().await.insert(id, peer);
        log::info!("add_peer finsh...");
    }

    async fn get_peer(&self, peer_id: String) -> Option<Arc<dyn Peer + Send + Sync>> {
        let rv = self.peers.lock().await.get(&peer_id).unwrap().clone();
        Some(rv)
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

    async fn remove_peer(&self, peer: Arc<dyn Peer + Send + Sync>) {
        let id = peer.id().await;

        let mut peers = self.peers.lock().await;
        if peers.get(&id).is_some() {
            peers.remove(&id);
        }

        let peer_count = peers.len() + self.relay_peers.lock().await.len();
        if peer_count == 0 {
            self.close().await;
        }
    }

    async fn add_data_channel(&self, owner: String, dc: Arc<RTCDataChannel>) {
        let label = dc.label().to_string();

        let data_channels_out = self.get_data_channels(owner.clone(), label.clone()).await;
        for lbl in &*self.fanout_data_channels.lock().await {
            if label == lbl.clone() {
                dc.on_message(Box::new(move |msg: DataChannelMessage| {
                    let data_channels_in = data_channels_out.clone();
                    Box::pin(async move {
                        SessionLocal::fanout_message(data_channels_in, msg).await;
                    })
                }));
                return;
            }
        }
        self.fanout_data_channels.lock().await.push(label.clone());
        let peer_owner = self.get_peer(owner.clone()).await.unwrap();

        if let Some(subscriber) = peer_owner.subscriber().await {
            subscriber
                .register_data_channel(label.clone(), dc.clone())
                .await;
        }

        let data_channels_out_2 = data_channels_out.clone();

        dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let data_channels_in = data_channels_out.clone();
            Box::pin(async move {
                SessionLocal::fanout_message(data_channels_in, msg).await;
            })
        }));

        for peer in &self.peers().await {
            if peer.id().await == owner || peer.subscriber().await.is_none() {
                continue;
            }

            if let Some(publisher) = peer.publisher().await {
                if publisher.relayed() {
                    //todo
                }
            }

            let subscriber = peer.subscriber().await.unwrap();
            if let Ok(channel) = subscriber.add_data_channel_2(label.clone()).await {
                let data_channels_out_3 = data_channels_out_2.clone();
                channel.on_message(Box::new(move |msg: DataChannelMessage| {
                    let data_channels_in = data_channels_out_3.clone();
                    Box::pin(async move {
                        SessionLocal::fanout_message(data_channels_in, msg).await;
                    })
                }));
                //todo
            } else {
                continue;
            }
            log::info!("AddDataChannel Negotiate");
            if let Err(err) = subscriber.negotiate().await {
                log::error!("negotiate error:{}", err);
            }
        }
    }

    async fn publish(
        &self,
        router: Arc<dyn Router + Send + Sync>,
        r: Arc<dyn Receiver + Send + Sync>,
    ) {
        for peer in self.peers().await {
            let subscriber = peer.subscriber().await;
            if router.id() == peer.id().await || subscriber.is_none() {
                continue;
            }
            log::info!(
                "PeerLocal Publishing track to peer, peer_id: {}",
                peer.id().await
            );
            if router
                .add_down_tracks(peer.subscriber().await.unwrap(), Some(r.clone()))
                .await
                .is_err()
            {
                continue;
            }
        }
    }
    async fn subscribe(&self, peer: Arc<dyn Peer + Send + Sync>) {
        log::info!("subscribe...");

        let fanout_data_channels = self.fanout_data_channels.lock().await;
        for label in &*fanout_data_channels {
            if let Some(subscriber) = peer.subscriber().await {
                match subscriber.add_data_channel_2(label.clone()).await {
                    Ok(dc) => {
                        let data_channels_out =
                            self.get_data_channels(peer.id().await, label.clone()).await;

                        dc.on_message(Box::new(move |msg: DataChannelMessage| {
                            let data_channels_in = data_channels_out.clone();
                            Box::pin(async move {
                                SessionLocal::fanout_message(data_channels_in, msg).await;
                            })
                        }));
                        //todo
                    }

                    Err(_) => {
                        continue;
                    }
                }
            }
        }

        let peers = self.peers.lock().await;

        for (_, cur_peer) in &*peers {
            //	Logger.V(0).Info("Subscribe to publisher streams...........", "peer_id", p.ID())

            if cur_peer.id().await == peer.id().await || cur_peer.publisher().await.is_none() {
                continue;
            }
            log::info!(
                "PeerLocal Subscribe to publisher streams , cur_peer_id:{} , peer_id:{}",
                cur_peer.id().await,
                peer.id().await
            );
            let router = cur_peer.publisher().await.unwrap().get_router();
            if router
                .add_down_tracks(peer.subscriber().await.unwrap(), None)
                .await
                .is_err()
            {
                continue;
            }
        }

        //todo
        log::info!("Subscribe Negotiate");
        if let Err(err) = peer.subscriber().await.unwrap().negotiate().await {
            log::error!("negotiate error: {}", err);
        }
    }

    async fn fanout_message(&self, origin: String, label: String, msg: DataChannelMessage) {
        let data_channels = self.get_data_channels(origin, label).await;
        SessionLocal::fanout_message(data_channels, msg).await;
    }

    async fn get_data_channels(&self, peer_id: String, label: String) -> Vec<Arc<RTCDataChannel>> {
        let mut data_channels: Vec<Arc<RTCDataChannel>> = Vec::new();

        let peers = self.peers.lock().await;
        for (k, v) in &*peers {
            if k.clone() == peer_id {
                continue;
            }

            if let Some(subscriber) = v.subscriber().await {
                if let Some(dc) = subscriber.data_channel(label.clone()).await {
                    if dc.ready_state() == RTCDataChannelState::Open {
                        data_channels.push(dc);
                    }
                }
            }
        }

        // todo
        // for (k, v) in &*self.relay_peers.lock().await {
        //     if let Some(dc) = v.data_channel(label.clone()) {

        //     }
        // }

        data_channels
    }

    async fn peers(&self) -> Vec<Arc<dyn Peer + Send + Sync>> {
        let mut peers: Vec<Arc<dyn Peer + Send + Sync>> = Vec::new();
        let peers_val = self.peers.lock().await;
        for (_, v) in &*peers_val {
            peers.push(v.clone());
        }
        peers
    }
    async fn relay_peers(&self) -> Vec<Arc<RelayPeer>> {
        let mut relay_peers: Vec<Arc<RelayPeer>> = Vec::new();
        for (_, v) in &*self.relay_peers.lock().await {
            relay_peers.push(v.clone());
        }
        relay_peers
    }

    async fn on_close(&self, f: OnCloseFn) {
        let mut handler = self.on_close_handler.lock().await;
        *handler = Some(f);
    }
}
