use std::collections::HashMap;

use webrtc::api;
use webrtc::api::media_engine::MediaEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::peer_connection::RTCPeerConnection;

use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::down_track::DownTrack;
use super::media_engine;
use super::sfu::WebRTCTransportConfig;

use anyhow::Result;

pub const API_CHANNEL_LABEL: &'static str = "rsfu";

pub type NegotiateFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

pub struct Subscriber {
    id: String,
    pc: Arc<RTCPeerConnection>,
    me: MediaEngine,

    tracks: HashMap<String, Vec<DownTrack>>,
    channels: HashMap<String, Vec<RTCDataChannel>>,
    candidates: Vec<RTCIceCandidateInit>,
    negotiate: Arc<Mutex<Option<NegotiateFn>>>,
    no_auto_subscribe: bool,
}

impl Subscriber {
    async fn new(id: String, cfg: WebRTCTransportConfig) -> Result<Subscriber> {
        let me = media_engine::get_subscriber_media_engine()?;

        let api = api::APIBuilder::new()
            .with_media_engine(me)
            .with_setting_engine(cfg.setting)
            .build();

        let pc = api.new_peer_connection(cfg.configuration).await?;

        let subscriber = Subscriber {
            id,
            pc: Arc::new(pc),
            me: media_engine::get_subscriber_media_engine()?,
            tracks: HashMap::new(),
            channels: HashMap::new(),
            candidates: Vec::new(),
            negotiate: Arc::new(Mutex::new(None)),
            no_auto_subscribe: false,
        };

        subscriber.on_ice_connection_state_change().await;

        Ok(subscriber)
    }

    async fn on_ice_connection_state_change(&self) {
        let pc_out = Arc::clone(&self.pc);
        self.pc
            .on_ice_connection_state_change(Box::new(move |ice_state: RTCIceConnectionState| {
                let pc_in = Arc::clone(&pc_out);

                Box::pin(async move {
                    match ice_state {
                        RTCIceConnectionState::Failed | RTCIceConnectionState::Closed => {
                            pc_in.close().await;
                        }
                        _ => {}
                    }
                })
            }))
            .await;
    }

    async fn close(&mut self) -> Result<(), webrtc::Error> {
        self.pc.close().await
    }

    //async fn add_data_channel(peer: Pee)
}
