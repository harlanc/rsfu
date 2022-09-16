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
    fn session(&self) -> Option<Arc<Mutex<dyn Session + Send + Sync>>>;
    // fn publisher() -> Arc<Publisher>;
    fn subscriber(&self) -> Option<Arc<Mutex<Subscriber>>>;
    // fn close() -> Result<()>;
    // fn send_data_channel_message(label: String, msg: Bytes) -> Result<()>;

    // async fn add_peer(self);

    // fn as_peer(&self) -> &(dyn Peer + Send + Sync);
}

struct JoinConfig {
    pub no_publish: bool,
    pub no_subscribe: bool,
    pub no_auto_subscribe: bool,
}

pub trait SessionProvider {
    fn get_session(
        &mut self,
        sid: String,
    ) -> (
        Option<Arc<Mutex<dyn Session + Send + Sync>>>,
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
struct PeerLocal {
    id: String,
    session: Option<Arc<Mutex<dyn Session + Send + Sync>>>,
    closed: Arc<AtomicBool>,
    provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>,
    publisher: Option<Arc<Mutex<Publisher>>>,
    subscriber: Option<Arc<Mutex<Subscriber>>>,

    on_offer_handler: Arc<Mutex<Option<OnOfferFn>>>,
    on_ice_candidate: Arc<Mutex<Option<OnIceCandidateFn>>>,
    on_ice_connection_state_change: Arc<Mutex<Option<OnIceConnectionStateChangeFn>>>,

    remote_answer_pending: Arc<AtomicBool>,
    negotiation_pending: Arc<AtomicBool>,
}
#[async_trait]
impl Peer for PeerLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn session(&self) -> Option<Arc<Mutex<dyn Session + Send + Sync>>> {
        self.session.clone()
    }

    fn subscriber(&self) -> Option<Arc<Mutex<Subscriber>>> {
        self.subscriber.clone()
    }

    // fn as_peer(&self) -> &(dyn Peer + Send + Sync) {
    //     self as &(dyn Peer + Send + Sync)
    // }
}

fn NewPeer() -> (impl Peer + Send + Sync) {
    let p = PeerLocal::new(Arc::new(Mutex::new(SProvider::new())));

    p
}

struct SProvider {}

impl SProvider {
    fn new() -> Self {
        SProvider {}
    }
}

impl SessionProvider for SProvider {
    fn get_session(
        &mut self,
        sid: String,
    ) -> (
        Option<Arc<Mutex<dyn Session + Send + Sync>>>,
        Arc<WebRTCTransportConfig>,
    ) {
        return (None, Arc::new(WebRTCTransportConfig::default()));
    }
}

impl PeerLocal {
    fn new(provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>) -> Self {
        PeerLocal {
            id: String::from(""),
            session: None,
            closed: Arc::new(AtomicBool::new(false)),
            provider,
            publisher: None,
            subscriber: None,

            on_offer_handler: Arc::new(Mutex::new(None)),
            on_ice_candidate: Arc::new(Mutex::new(None)),
            on_ice_connection_state_change: Arc::new(Mutex::new(None)),

            remote_answer_pending: Arc::new(AtomicBool::new(false)),
            negotiation_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn add_peer(self: &Arc<Self>) {
        // let s = Arc::new(Box::new(self) as Box<dyn Peer + Send + Sync>);
        if let Some(session) = &self.session {
            session.lock().await.add_peer(Arc::clone(self)); //as &Arc<dyn Peer + Send + Sync>));
        }
    }

    async fn join(&mut self, sid: String, uid: String, cfg: JoinConfig) -> Result<()> {
        if !self.session.is_none() {
            return Err(Error::ErrTransportExists.into());
        }

        let mut uuid: String = uid.clone();
        if uid == String::from("") {
            uuid = Uuid::new_v4().to_string();
        }

        self.id = uuid;

        let (s, webrtc_transport_cfg) = self.provider.lock().await.get_session(sid);

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

        self.session = s;

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

            let on_ice_candidate_out = self.on_ice_candidate.clone();
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

        if !cfg.no_publish {
            let mut publisher = Publisher::new(
                self.id.clone(),
                self.session.as_ref().unwrap().clone(),
                config_clone,
            )
            .await?;

            if !cfg.no_subscribe {
                let session = self.session.as_ref().unwrap().lock().await;
                for dc in session.get_data_channel_middlewares() {
                    if let Some(subscriber) = &self.subscriber {
                        subscriber.lock().await.add_data_channel(dc).await?;
                    }
                    // let subscriber = self.subscriber.unwrap().lock().await;
                }
            }

            let on_ice_candidate_out = self.on_ice_candidate.clone();
            let closed_out_1 = self.closed.clone();

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

            self.publisher = Some(Arc::new(Mutex::new(publisher)));
        }

        // if let Some(session) = self.session {
        //     session.lock().await.add_peer(Arc::new(self.as_peer()));
        // }

        Ok(())
    }
}
