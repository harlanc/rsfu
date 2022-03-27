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
    fn session(&self) -> Arc<dyn Session + Send + Sync>;
    // fn publisher() -> Arc<Publisher>;
    // fn subscriber() -> Arc<Subscriber>;
    // fn close() -> Result<()>;
    // fn send_data_channel_message(label: String, msg: Bytes) -> Result<()>;
}

struct JoinConfig {
    no_publish: bool,
    no_subscribe: bool,
    no_auto_subscribe: bool,
}

pub trait SessionProvider {
    fn get_session(
        &mut self,
        sid: String,
    ) -> (Arc<dyn Session + Send + Sync>, WebRTCTransportConfig);
}

struct ChannelAPIMessage {
    method: String,
    params: Vec<String>,
}

#[derive(Default)]
struct PeerLocal {
    id: String,
    session: Option<Arc<dyn Session + Send + Sync>>,
    closed: AtomicBool,
    provider: Arc<dyn SessionProvider + Send + Sync>,
    publisher: Publisher,
    subscriber: Subscriber,

    on_offer_handler: Arc<Mutex<Option<OnOfferFn>>>,
    on_ice_candidate: Arc<Mutex<Option<OnIceCandidateFn>>>,
    on_ice_connection_state_change: Arc<Mutex<Option<OnIceConnectionStateChangeFn>>>,

    remote_answer_pending: bool,
    negotiation_pending: bool,
}

impl PeerLocal {
    fn new(provider: Arc<dyn SessionProvider + Send + Sync>) -> Self {
        PeerLocal {
            provider,
            ..Default::default()
        }
    }

    fn join(&mut self, sid: String, uid: String, cfg: JoinConfig) -> Result<()> {
        if self.session != None {
            return Err(Error::ErrTransportExists.into());
        }

        let uuid: String;
        if uid == String::from("") {
            uuid = Uuid::new_v4().to_string();
        }

        self.id = uuid;

        let (s, cfg) = self.provider.get_session(sid);

        self.session = s;

        Ok(())
    }
}

impl Peer for PeerLocal {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn session(&self) -> Arc<dyn Session + Send + Sync> {
        self.session.clone()
    }
}
