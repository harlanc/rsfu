use super::publisher::Publisher;
use super::sfu::WebRTCTransportConfig;
use super::subscriber::{self, Subscriber};
use super::{publisher::PublisherTrack, session::Session};
use crate::buffer::factory::AtomicFactory;
use async_trait::async_trait;
use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::signaling_state::RTCSignalingState;

use super::errors::Error;
// use super::errors::Result;

use anyhow::Result;
use serde::{Deserialize, Serialize};

const PUBLISHER: u8 = 0;
const SUBSCRIBER: u8 = 1;

pub type OnOfferFn = Box<
    dyn (FnMut(RTCSessionDescription) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnIceCandidateFn = Box<
    dyn (FnMut(RTCIceCandidateInit, u8) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnIceConnectionStateChangeFn = Box<
    dyn (FnMut(RTCIceConnectionState) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

#[async_trait]
pub trait Peer {
    fn id(&self) -> String;
    fn session(&self) -> Option<Arc<dyn Session + Send + Sync>>;
    fn publisher(&self) -> Option<Arc<Mutex<Publisher>>>;
    fn subscriber(&self) -> Option<Arc<Mutex<Subscriber>>>;
    //fn close() -> Result<()>;
    //no used now
    //fn send_data_channel_message(label: String, msg: Bytes) -> Result<()>;

    // async fn add_peer(self);

    // fn as_peer(&self) -> &(dyn Peer + Send + Sync);
}

#[derive(Clone, Default)]
pub struct JoinConfig {
    pub no_publish: bool,
    pub no_subscribe: bool,
    pub no_auto_subscribe: bool,
}

#[async_trait]
pub trait SessionProvider {
    async fn get_session(
        &self,
        sid: String,
    ) -> (
        Option<Arc<dyn Session + Send + Sync>>,
        Arc<WebRTCTransportConfig>,
    );
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelAPIMessage {
    #[serde(rename = "method")]
    pub method: String,
    #[serde(rename = "parameters")]
    pub params: Vec<String>,
}

// #[derive(Default)]
pub struct PeerLocal {
    id: String,
    session: Option<Arc<dyn Session + Send + Sync>>,
    closed: Arc<AtomicBool>,
    provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>,
    publisher: Option<Arc<Mutex<Publisher>>>,
    subscriber: Option<Arc<Mutex<Subscriber>>>,

    pub on_offer_handler: Arc<Mutex<Option<OnOfferFn>>>,
    pub on_ice_candidate_handler: Arc<Mutex<Option<OnIceCandidateFn>>>,
    on_ice_connection_state_change: Arc<Mutex<Option<OnIceConnectionStateChangeFn>>>,

    remote_answer_pending: Arc<AtomicBool>,
    negotiation_pending: Arc<AtomicBool>,
}
#[async_trait]
impl Peer for PeerLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn session(&self) -> Option<Arc<dyn Session + Send + Sync>> {
        self.session.clone()
    }

    fn subscriber(&self) -> Option<Arc<Mutex<Subscriber>>> {
        self.subscriber.clone()
    }

    fn publisher(&self) -> Option<Arc<Mutex<Publisher>>> {
        self.publisher.clone()
    }

    // fn as_peer(&self) -> &(dyn Peer + Send + Sync) {
    //     self as &(dyn Peer + Send + Sync)
    // }
}

// fn NewPeer() //-> (impl Peer + Send + Sync)
// {
//     //     let p = PeerLocal::new(Arc::new(Mutex::new(SProvider::new())));

//     //     p

//     let p = Arc::new(PeerLocal::new(Arc::new(Mutex::new(SProvider::new()))));

//     let a = Arc::clone(&p) as Arc<dyn Peer + Send + Sync>;
// }

// struct SProvider {}

// impl SProvider {
//     fn new() -> Self {
//         SProvider {}
//     }
// }

// impl SessionProvider for SProvider {
//     async fn get_session(
//         &mut self,
//         sid: String,
//     ) -> (
//         Option<Arc<Mutex<dyn Session + Send + Sync>>>,
//         Arc<WebRTCTransportConfig>,
//     ) {
//         return (None, Arc::new(WebRTCTransportConfig::default()));
//     }
// }

impl PeerLocal {
    pub fn new(provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>) -> Self {
        let peer_local = PeerLocal {
            id: String::from(""),
            session: None,
            closed: Arc::new(AtomicBool::new(false)),
            provider,
            publisher: None,
            subscriber: None,

            on_offer_handler: Arc::new(Mutex::new(None)),
            on_ice_candidate_handler: Arc::new(Mutex::new(None)),
            on_ice_connection_state_change: Arc::new(Mutex::new(None)),

            remote_answer_pending: Arc::new(AtomicBool::new(false)),
            negotiation_pending: Arc::new(AtomicBool::new(false)),
        };

        peer_local
    }

    // pub async fn join_after(self: &Arc<Mutex<Self>>) {
    //     // let s = Arc::new(Box::new(self) as Box<dyn Peer + Send + Sync>);

    //     // let session = self.session.take();
    //     if let Some(session) = &self.session {
    //         session
    //             .lock()
    //             .await
    //             .add_peer(Arc::clone(self) as Arc<dyn Peer + Send + Sync>); //as &Arc<dyn Peer + Send + Sync>));

    //         session
    //             .lock()
    //             .await
    //             .subscribe(Arc::clone(self) as Arc<dyn Peer + Send + Sync>);
    //     }
    // }

    pub async fn on_offer(&self, f: OnOfferFn) {
        let mut handler = self.on_offer_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_ice_candidate(&self, f: OnIceCandidateFn) {
        let mut handler = self.on_ice_candidate_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn join(&mut self, sid: String, uid: String, cfg: JoinConfig) -> Result<()> {
        log::info!("join begin...");
        if !self.session.is_none() {
            return Err(Error::ErrTransportExists.into());
        }

        let mut uuid: String = uid.clone();
        if uid == String::from("") {
            uuid = Uuid::new_v4().to_string();
        }
        log::info!("join begin 0...");
        self.id = uuid;

        let mut provider = self.provider.lock().await;
        log::info!("join begin 0.01...");
        let (s, webrtc_transport_cfg) = provider.get_session(sid).await;
        log::info!("join begin 0.1...");
        let rtc_config_clone = RTCConfiguration {
            ice_servers: webrtc_transport_cfg.configuration.ice_servers.clone(),
            ..Default::default()
        };
        log::info!("join begin 1...");
        let config_clone = WebRTCTransportConfig {
            configuration: rtc_config_clone,
            setting: webrtc_transport_cfg.setting.clone(),
            Router: webrtc_transport_cfg.Router.clone(),
            factory: Arc::new(Mutex::new(AtomicFactory::new(1000, 1000))),
        };
        log::info!("join begin 2...");
        self.session = s;
        log::info!("join 1...");
        if !cfg.no_subscribe {
            let subscriber = Arc::new(Mutex::new(
                Subscriber::new(self.id.clone(), webrtc_transport_cfg).await?,
            ));

            subscriber.lock().await.no_auto_subscribe = cfg.no_auto_subscribe;

            let remote_answer_pending_out = self.remote_answer_pending.clone();
            let negotiation_pending_out = self.negotiation_pending.clone();
            let closed_out = self.closed.clone();

            let sub = Arc::clone(&subscriber);
            let on_offer_handler_out = self.on_offer_handler.clone();

            subscriber
                .lock()
                .await
                .on_negotiate(Box::new(move || {
                    let remote_answer_pending_in = remote_answer_pending_out.clone();
                    let negotiation_pending_in = negotiation_pending_out.clone();
                    let closed_in = closed_out.clone();

                    let sub_in = sub.clone();
                    let on_offer_handler_in = on_offer_handler_out.clone();

                    Box::pin(async move {
                        if remote_answer_pending_in.load(Ordering::Relaxed) {
                            (*negotiation_pending_in).store(true, Ordering::Relaxed);
                            return Ok(());
                        }

                        let offer = sub_in.lock().await.create_offer().await?;
                        (*remote_answer_pending_in).store(true, Ordering::Relaxed);

                        if let Some(on_offer) = &mut *on_offer_handler_in.lock().await {
                            if !closed_in.load(Ordering::Relaxed) {
                                on_offer(offer).await;
                            }
                        }

                        Ok(())
                    })
                }))
                .await;

            let on_ice_candidate_out = self.on_ice_candidate_handler.clone();
            let closed_out_1 = self.closed.clone();
            subscriber
                .lock()
                .await
                .on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
                    let on_ice_candidate_in = on_ice_candidate_out.clone();
                    let closed_in = closed_out_1.clone();
                    Box::pin(async move {
                        if candidate.is_none() {
                            return;
                        }

                        if let Some(on_ice_candidate) = &mut *on_ice_candidate_in.lock().await {
                            if !closed_in.load(Ordering::Relaxed) {
                                if let Ok(val) = candidate.unwrap().to_json().await {
                                    on_ice_candidate(val, SUBSCRIBER).await;
                                }
                            }
                        }
                    })
                }))
                .await;

            self.subscriber = Some(subscriber);
        }
        log::info!("join 2...");
        if !cfg.no_publish {
            log::info!("join 3...");
            if !cfg.no_subscribe {
                log::info!("join 3111...");
                let session_mutex = self.session.as_ref().unwrap();
                log::info!("join 3112...");
                //let session_1 = session_mutex.lock();
                log::info!("join 3113...");
                // let session = session_1.await;
                log::info!("join 3.1...");
                for dc in &*session_mutex.get_data_channel_middlewares().lock().await {
                    log::info!("join 3.2...");
                    if let Some(subscriber) = &self.subscriber {
                        log::info!("join 3.3...");
                        subscriber.lock().await.add_data_channel(dc.clone()).await?;
                    }
                    // let subscriber = self.subscriber.unwrap().lock().await;
                }
            }
            log::info!("join 4...");
            let on_ice_candidate_out = self.on_ice_candidate_handler.clone();
            let closed_out_1 = self.closed.clone();

            let mut publisher = Publisher::new(
                self.id.clone(),
                self.session.as_ref().unwrap().clone(),
                config_clone,
            )
            .await?;

            publisher
                .on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
                    let on_ice_candidate_in = on_ice_candidate_out.clone();
                    let closed_in = closed_out_1.clone();
                    Box::pin(async move {
                        if candidate.is_none() {
                            return;
                        }

                        if let Some(on_ice_candidate) = &mut *on_ice_candidate_in.lock().await {
                            if !closed_in.load(Ordering::Relaxed) {
                                if let Ok(val) = candidate.unwrap().to_json().await {
                                    on_ice_candidate(val, PUBLISHER).await;
                                }
                            }
                        }
                    })
                }))
                .await;
            log::info!("join 5...");
            let on_ice_connection_state_change_out = self.on_ice_connection_state_change.clone();
            let closed_out_2 = self.closed.clone();
            publisher
                .on_ice_connection_state_change(Box::new(move |state: RTCIceConnectionState| {
                    let on_ice_connection_state_change_in =
                        on_ice_connection_state_change_out.clone();
                    let closed_in = closed_out_2.clone();

                    Box::pin(async move {
                        if let Some(on_ice_connection_state_change) =
                            &mut *on_ice_connection_state_change_in.lock().await
                        {
                            if !closed_in.load(Ordering::Relaxed) {
                                on_ice_connection_state_change(state).await;
                            }
                        }
                    })
                }))
                .await;
            log::info!("join 6...");
            self.publisher = Some(Arc::new(Mutex::new(publisher)));
        }

        Ok(())
    }

    pub async fn answer(&mut self, sdp: RTCSessionDescription) -> Result<RTCSessionDescription> {
        if let Some(publisher) = &self.publisher {
            if publisher.lock().await.signaling_state() != RTCSignalingState::Stable {
                return Err(Error::ErrOfferIgnored.into());
            }

            publisher.lock().await.answer(sdp).await
        } else {
            return Err(Error::ErrNoTransportEstablished.into());
        }
    }

    pub async fn set_remote_description(&mut self, sdp: RTCSessionDescription) -> Result<()> {
        if let Some(subscriber) = &self.subscriber {
            log::info!("peer local got answer, peer id:{}", self.id);
            subscriber.lock().await.set_remote_description(sdp).await?;
            self.remote_answer_pending.store(false, Ordering::Relaxed);

            if self.negotiation_pending.load(Ordering::Relaxed) {
                self.negotiation_pending.store(false, Ordering::Relaxed);
                subscriber.lock().await.negotiate().await;
            }
        } else {
            return Err(Error::ErrNoTransportEstablished.into());
        }

        Ok(())
    }

    pub async fn trickle(&mut self, candidate: RTCIceCandidateInit, target: u8) -> Result<()> {
        if self.subscriber.is_none() || self.publisher.is_none() {
            return Err(Error::ErrNoTransportEstablished.into());
        }

        log::info!("peer local trickle, peer id:{}", self.id);
        match target {
            PUBLISHER => {
                self.publisher
                    .as_ref()
                    .unwrap()
                    .lock()
                    .await
                    .add_ice_candidata(candidate)
                    .await?;
            }
            SUBSCRIBER => {
                self.subscriber
                    .as_ref()
                    .unwrap()
                    .lock()
                    .await
                    .add_ice_candidate(candidate)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn send_data_channel_message(&mut self, label: String, msg: String) -> Result<()> {
        if self.subscriber.is_none() {
            return Err(Error::ErrNoSubscriber.into());
        }

        let dc = self
            .subscriber
            .as_ref()
            .unwrap()
            .lock()
            .await
            .data_channel(label);

        if dc.is_none() {
            return Err(Error::ErrDataChannelNotExists.into());
        }

        dc.unwrap().send_text(msg).await?;

        Ok(())
    }

    async fn close(self: &Arc<Self>) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.closed.store(true, Ordering::Relaxed);

        if let Some(session) = &self.session {
            session
                .remove_peer(Arc::clone(self) as Arc<dyn Peer + Send + Sync>)
                .await;
        }

        if let Some(publisher) = &self.publisher {
            publisher.lock().await.close().await;
        }
        if let Some(subscriber) = &self.subscriber {
            subscriber.lock().await.close().await?;
        }
        Ok(())
    }

    fn subscriber(self) -> Option<Arc<Mutex<Subscriber>>> {
        self.subscriber
    }
    fn publisher(self) -> Option<Arc<Mutex<Publisher>>> {
        self.publisher
    }
    fn session(self) -> Option<Arc<dyn Session + Send + Sync>> {
        self.session
    }
    fn id(self) -> String {
        self.id
    }
}
