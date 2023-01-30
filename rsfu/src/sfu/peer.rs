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
    async fn id(&self) -> String;
    async fn session(&self) -> Option<Arc<dyn Session + Send + Sync>>;
    async fn publisher(&self) -> Option<Arc<Publisher>>;
    async fn subscriber(&self) -> Option<Arc<Subscriber>>;
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
    id: Arc<Mutex<String>>,
    session: Arc<Mutex<Option<Arc<dyn Session + Send + Sync>>>>,
    closed: Arc<AtomicBool>,
    provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>,
    publisher: Arc<Mutex<Option<Arc<Publisher>>>>,
    subscriber: Arc<Mutex<Option<Arc<Subscriber>>>>,

    pub on_offer_handler: Arc<Mutex<Option<OnOfferFn>>>,
    pub on_ice_candidate_handler: Arc<Mutex<Option<OnIceCandidateFn>>>,
    on_ice_connection_state_change: Arc<Mutex<Option<OnIceConnectionStateChangeFn>>>,

    remote_answer_pending: Arc<AtomicBool>,
    negotiation_pending: Arc<AtomicBool>,
}
#[async_trait]
impl Peer for PeerLocal {
    async fn id(&self) -> String {
        self.id.lock().await.clone()
    }

    async fn session(&self) -> Option<Arc<dyn Session + Send + Sync>> {
        self.session.lock().await.clone()
    }

    async fn subscriber(&self) -> Option<Arc<Subscriber>> {
        self.subscriber.lock().await.clone()
    }

    async fn publisher(&self) -> Option<Arc<Publisher>> {
        self.publisher.lock().await.clone()
        // fn as_peer(&self) -> &(dyn Peer + Send + Sync) {
        //     self as &(dyn Peer + Send + Sync)
        // }
    }
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
    pub fn new(provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>) -> PeerLocal {
        let peer_local = PeerLocal {
            id: Arc::new(Mutex::new(String::from(""))),
            session: Arc::new(Mutex::new(None)),
            closed: Arc::new(AtomicBool::new(false)),
            provider,
            publisher: Arc::new(Mutex::new(None)),
            subscriber: Arc::new(Mutex::new(None)),

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

    pub async fn join(self: &Arc<Self>, sid: String, uid: String, cfg: JoinConfig) -> Result<()> {
        log::info!("join begin...");
        if self.session.lock().await.is_some() {
            return Err(Error::ErrTransportExists.into());
        }

        let mut uuid: String = uid.clone();
        if uid == String::from("") {
            uuid = Uuid::new_v4().to_string();
        }

        // let id = &mut *self.id.lock().await;
        // id = &mut uuid;

        *self.id.lock().await = uuid.clone();

        let provider = self.provider.lock().await;

        let (cur_session, webrtc_transport_cfg) = provider.get_session(sid.clone()).await;

        let rtc_config_clone = RTCConfiguration {
            ice_servers: webrtc_transport_cfg.configuration.ice_servers.clone(),
            ..Default::default()
        };

        let config_clone = WebRTCTransportConfig {
            configuration: rtc_config_clone,
            setting: webrtc_transport_cfg.setting.clone(),
            Router: webrtc_transport_cfg.Router.clone(),
            factory: Arc::new(Mutex::new(AtomicFactory::new(1000, 1000))),
        };

        *self.session.lock().await = cur_session.clone();

        if !cfg.no_subscribe {
            let mut raw_subscriber =
                Subscriber::new(self.id.lock().await.clone(), webrtc_transport_cfg).await?;
            raw_subscriber.no_auto_subscribe = cfg.no_auto_subscribe;
            let subscriber = Arc::new(raw_subscriber);

            let remote_answer_pending_out = self.remote_answer_pending.clone();
            let negotiation_pending_out = self.negotiation_pending.clone();
            let closed_out = self.closed.clone();

            let sub = Arc::clone(&subscriber);
            let on_offer_handler_out = self.on_offer_handler.clone();
            let uuid_out = uuid.clone();
            println!("on_offer 0");

            subscriber
                .on_negotiate(Box::new(move || {
                    let remote_answer_pending_in = remote_answer_pending_out.clone();
                    let negotiation_pending_in = negotiation_pending_out.clone();
                    let closed_in = closed_out.clone();
                    let uuid_clone = uuid_out.clone();

                    let sub_in = sub.clone();
                    let on_offer_handler_in = on_offer_handler_out.clone();
                    println!("on_offer 1");
                    Box::pin(async move {
                        if remote_answer_pending_in.load(Ordering::Relaxed) {
                            println!("on_offer 1.1");
                            (*negotiation_pending_in).store(true, Ordering::Relaxed);
                            return Ok(());
                        }
                        println!("on_offer 1.2");

                        let offer = sub_in.create_offer().await?;
                        (*remote_answer_pending_in).store(true, Ordering::Relaxed);
                        println!("on_offer 2");
                        if let Some(on_offer) = &mut *on_offer_handler_in.lock().await {
                            if !closed_in.load(Ordering::Relaxed) {
                                log::info!("PeerLocal Send offer, peer_id: {}", uuid_clone);
                                on_offer(offer).await;
                            }
                        }

                        Ok(())
                    })
                }))
                .await;

            let on_ice_candidate_out = self.on_ice_candidate_handler.clone();
            let closed_out_1 = self.closed.clone();
            subscriber.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
                let on_ice_candidate_in = on_ice_candidate_out.clone();
                let closed_in = closed_out_1.clone();
                Box::pin(async move {
                    if candidate.is_none() {
                        return;
                    }

                    if let Some(on_ice_candidate) = &mut *on_ice_candidate_in.lock().await {
                        if !closed_in.load(Ordering::Relaxed) {
                            if let Ok(val) = candidate.unwrap().to_json() {
                                on_ice_candidate(val, SUBSCRIBER).await;
                            }
                        }
                    }
                })
            }));
            *self.subscriber.lock().await = Some(subscriber);
        }

        if !cfg.no_publish {
            if !cfg.no_subscribe {
                if let Some(session) = cur_session.clone() {
                    for dc in &*session.get_data_channel_middlewares().lock().await {
                        if let Some(sub) = &*self.subscriber.lock().await {
                            sub.add_data_channel(dc.clone()).await?;
                        }
                    }
                }
                //let session_mutex = self.session.lock().await.as_ref().unwrap();
            }
            let on_ice_candidate_out = self.on_ice_candidate_handler.clone();
            let closed_out_1 = self.closed.clone();

            let publisher = Arc::new(
                Publisher::new(
                    self.id.lock().await.clone(),
                    cur_session.clone().unwrap(),
                    config_clone,
                )
                .await?,
            );

            publisher.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
                let on_ice_candidate_in = on_ice_candidate_out.clone();
                let closed_in = closed_out_1.clone();
                Box::pin(async move {
                    if candidate.is_none() {
                        return;
                    }

                    if let Some(on_ice_candidate) = &mut *on_ice_candidate_in.lock().await {
                        if !closed_in.load(Ordering::Relaxed) {
                            if let Ok(val) = candidate.unwrap().to_json() {
                                on_ice_candidate(val, PUBLISHER).await;
                            }
                        }
                    }
                })
            }));

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
            *self.publisher.lock().await = Some(publisher);
        }

        cur_session.clone().unwrap().add_peer(self.clone()).await;

        //Logger.V(0).Info("PeerLocal join SessionLocal", "peer_id", p.id, "session_id", sid)

        log::info!(
            "PeerLocal join SessionLocal ,peer_id: {} session_id: {}",
            uuid,
            sid
        );

        if !cfg.no_subscribe {
            cur_session.clone().unwrap().subscribe(self.clone()).await;
        }

        Ok(())
    }

    pub async fn answer(&self, sdp: RTCSessionDescription) -> Result<RTCSessionDescription> {
        if let Some(publisher) = &*self.publisher.lock().await {
            //Logger.V(0).Info("PeerLocal got offer", "peer_id", p.id)
            log::info!("PeerLocal got offer, peer_id :{}", self.id().await);
            if publisher.signaling_state() != RTCSignalingState::Stable {
                return Err(Error::ErrOfferIgnored.into());
            }

            let rv = publisher.answer(sdp).await;
            log::info!("PeerLocal send answer, peer_id :{}", self.id().await);
            rv
        } else {
            return Err(Error::ErrNoTransportEstablished.into());
        }
    }

    pub async fn set_remote_description(&self, sdp: RTCSessionDescription) -> Result<()> {
        if let Some(subscriber) = &*self.subscriber.lock().await {
            log::info!("PeerLocal got answer, peer id:{}", self.id.lock().await);
            subscriber.set_remote_description(sdp).await?;
            self.remote_answer_pending.store(false, Ordering::Relaxed);

            if self.negotiation_pending.load(Ordering::Relaxed) {
                self.negotiation_pending.store(false, Ordering::Relaxed);
                log::info!("set_remote_description 2 Negotiate");
                subscriber.negotiate().await?;
            }
        } else {
            return Err(Error::ErrNoTransportEstablished.into());
        }

        Ok(())
    }

    pub async fn trickle(&self, candidate: RTCIceCandidateInit, target: u8) -> Result<()> {
        let subscriber = self.subscriber.lock().await;
        let publisher = self.publisher.lock().await;
        if subscriber.is_none() || publisher.is_none() {
            return Err(Error::ErrNoTransportEstablished.into());
        }

        log::info!("PeerLocal trickle, peer_id:{}", self.id.lock().await);
        match target {
            PUBLISHER => {
                if let Some(publisher) = &*publisher {
                    publisher.add_ice_candidata(candidate).await?;
                }
            }
            SUBSCRIBER => {
                if let Some(subscriber) = &*subscriber {
                    subscriber.add_ice_candidate(candidate).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn send_data_channel_message(&mut self, label: String, msg: String) -> Result<()> {
        let subscriber = self.subscriber.lock().await;
        if subscriber.is_none() {
            return Err(Error::ErrNoSubscriber.into());
        }

        let dc = subscriber.as_ref().unwrap().data_channel(label).await;

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

        if let Some(session) = &*self.session.lock().await {
            session
                .remove_peer(Arc::clone(self) as Arc<dyn Peer + Send + Sync>)
                .await;
        }

        if let Some(publisher) = &*self.publisher.lock().await {
            publisher.close().await;
        }

        if let Some(subscriber) = &*self.subscriber.lock().await {
            subscriber.close().await?;
        }

        Ok(())
    }

    // fn subscriber(self) -> Arc<Mutex<Subscriber>> {
    //     self.subscriber
    // }
    // fn publisher(self) -> Option<Arc<Publisher>> {
    //     self.publisher
    // }
    // fn session(self) -> Option<Arc<dyn Session + Send + Sync>> {
    //     self.session.lock
    // }
    async fn id(self) -> String {
        self.id.lock().await.clone()
    }
}
