use super::publisher::Publisher;
use super::sfu::WebRTCTransportConfig;
use super::subscriber::Subscriber;
use super::{publisher::PublisherTrack, session::Session};
use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use uuid::Uuid;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::sdp::description::session::SessionDescription;

use super::errors::Error;
use super::errors::Result;

const PUBLISHER: u8 = 0;
const SUBSCRIBER: u8 = 1;

pub type OnOfferFn = Box<
    dyn (FnMut(SessionDescription) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnIceCandidateFn = Box<
    dyn (FnMut(RTCIceCandidateInit) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnIceConnectionStateChangeFn = Box<
    dyn (FnMut(RTCIceConnectionState) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub trait Peer {
    fn id(&self) -> String;
    fn session(&self) -> Option<Arc<Mutex<dyn Session + Send + Sync>>>;
    // fn publisher() -> Arc<Publisher>;
    // fn subscriber() -> Arc<Subscriber>;
    // fn close() -> Result<()>;
    // fn send_data_channel_message(label: String, msg: Bytes) -> Result<()>;
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

struct ChannelAPIMessage {
    method: String,
    params: Vec<String>,
}

// #[derive(Default)]
struct PeerLocal {
    id: String,
    session: Option<Arc<Mutex<dyn Session + Send + Sync>>>,
    closed: AtomicBool,
    provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>,
    publisher: Option<Publisher>,
    subscriber: Option<Subscriber>,

    on_offer_handler: Arc<Mutex<Option<OnOfferFn>>>,
    on_ice_candidate: Arc<Mutex<Option<OnIceCandidateFn>>>,
    on_ice_connection_state_change: Arc<Mutex<Option<OnIceConnectionStateChangeFn>>>,

    remote_answer_pending: Arc<bool>,
    negotiation_pending: Arc<AtomicBool>,
}

impl PeerLocal {
    fn new(provider: Arc<Mutex<dyn SessionProvider + Send + Sync>>) -> Self {
        PeerLocal {
            id: String::from(""),
            session: None,
            closed: AtomicBool::new(false),
            provider,
            publisher: None,
            subscriber: None,

            on_offer_handler: Arc::new(Mutex::new(None)),
            on_ice_candidate: Arc::new(Mutex::new(None)),
            on_ice_connection_state_change: Arc::new(Mutex::new(None)),

            remote_answer_pending: Arc::new(false),
            negotiation_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn join(&mut self, sid: String, uid: String, cfg: JoinConfig) -> Result<()> {
        if self.session.is_none() {
            return Err(Error::ErrTransportExists.into());
        }

        let mut uuid: String = uid.clone();
        if uid == String::from("") {
            uuid = Uuid::new_v4().to_string();
        }

        self.id = uuid.clone();

        let (s, webrtc_transport_cfg) = self.provider.lock().await.get_session(sid);

        self.session = s;

        if !cfg.no_subscribe {
            let mut subscriber = Subscriber::new(uuid, webrtc_transport_cfg).await?;
            subscriber.no_auto_subscribe = cfg.no_auto_subscribe;

            let remote_answer_pending_out = self.remote_answer_pending.clone();
            let negotiation_pending_out = self.negotiation_pending.clone();
            subscriber
                .on_negotiate(Box::new(move || {
                    let remote_answer_pending_in = remote_answer_pending_out.clone();
                    let negotiation_pending_in = negotiation_pending_out.clone();

                    Box::pin(async move {
                        if *remote_answer_pending_in {
                            (*negotiation_pending_in).store(true, Ordering::Relaxed);
                            return;
                        }
                        
                    })
                }))
                .await;

            self.subscriber = Some(subscriber);
        }

        Ok(())
    }
}

impl Peer for PeerLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn session(&self) -> Option<Arc<Mutex<dyn Session + Send + Sync>>> {
        self.session.clone()
    }
}
