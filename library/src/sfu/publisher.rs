use super::router::Router;
use super::router::RouterLocal;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;

use super::media_engine;
use super::receiver::Receiver;
use crate::relay::relay::Peer;
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::{Mutex, MutexGuard};
use webrtc::api::APIBuilder;
use webrtc::api::API;
use webrtc::data_channel::RTCDataChannel;
use webrtc::track::track_remote::TrackRemote;

pub type OnIceConnectionStateChange =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub type OnPublisherTrack =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

// pub type OnTrackHdlrFn = Box<
//     dyn (FnMut(
//             Option<Arc<TrackRemote>>,
//             Option<Arc<RTCRtpReceiver>>,
//         ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
//         + Send
//         + Sync,
// >;
struct Publisher {
    id: String,

    pc: RTCPeerConnection,
    cfg: WebRTCTransportConfig,

    router: Arc<dyn Router + Send + Sync>,
    session: Arc<Box<dyn Session + Send + Sync>>,
    tracks: Vec<PublisherTrack>,

    relayed: AtomicBool,
    relay_peers: Vec<RelayPeer>,
    candidates: Vec<RTCIceCandidateInit>,

    on_ice_connection_state_change_hander: Arc<Mutex<Option<OnIceConnectionStateChange>>>,
    on_publisher_track: Arc<Mutex<Option<OnPublisherTrack>>>,

    close_once: Once,
}

pub struct RelayPeer {
    peer: Peer,
    data_channels: Vec<RTCDataChannel>,

    with_sr_reports: bool,
    relay_fanout_data_channels: bool,
}

pub(super) struct PublisherTrack {
    track: TrackRemote,
    receiver: Arc<dyn Receiver + Send + Sync>,

    // This will be used in the future for tracks that will be relayed as clients or servers
    // This is for SVC and Simulcast where you will be able to chose if the relayed peer just
    // want a single track (for recording/ processing) or get all the tracks (for load balancing)
    client_relay: bool,
}

impl Publisher {
    pub async fn new(
        id: String,
        session: Arc<Box<dyn Session + Send + Sync>>,
        cfg: WebRTCTransportConfig,
    ) -> Result<Self> {
        let me = media_engine::get_publisher_media_engine().await?;

        let setting_engine = cfg.setting.clone();

        let api = APIBuilder::new()
            .with_media_engine(me)
            .with_setting_engine(setting_engine)
            .build();

        let router = cfg.Router.clone();

        let pc = api.new_peer_connection(cfg.configuration).await?;

        let publisher = Publisher {
            id: id.clone(),
            pc,
            cfg,
            router: Arc::new(RouterLocal::new(id, session.clone(), router)),
            session: session,

            tracks: Vec::new(),

            relayed: AtomicBool::new(false),
            relay_peers: Vec::new(),
            candidates: Vec::new(),
            on_ice_connection_state_change_hander: Arc::default(),
            on_publisher_track: Arc::default(),
            close_once: Once::new(),
        };

        pc.on_track(Box::new(
            move |track: Option<Arc<TrackRemote>>, receiver: Option<Arc<RTCRtpReceiver>>| {
                Box::pin(async move {
                    if let Some(receiver_val) = receiver {
                        if let Some(track_val) = track {
                            publisher.router.add_receiver(
                                receiver_val,
                                track_val,
                                track_val.id().await,
                                track_val.stream_id().await,
                            );
                        }
                    }
                })
            },
        ))
        .await;

        Ok(publisher)
    }
}
