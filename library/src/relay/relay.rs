use std::clone;
use std::fs::OpenOptions;

use bytes::Bytes;
use serde_json::Map;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::api::API;
use webrtc::data::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data::data_channel::RTCDataChannel;
use webrtc::data::sctp_transport::sctp_transport_capabilities::SCTPTransportCapabilities;
use webrtc::data::sctp_transport::RTCSctpTransport;
use webrtc::media::dtls_transport::dtls_parameters::DTLSParameters;
use webrtc::media::dtls_transport::RTCDtlsTransport;
use webrtc::media::ice_transport::ice_parameters::RTCIceParameters;
use webrtc::media::ice_transport::ice_role::RTCIceRole;
use webrtc::media::ice_transport::RTCIceTransport;
use webrtc::media::rtp::rtp_codec::RTCRtpCodecParameters;
use webrtc::media::rtp::rtp_codec::RTCRtpParameters;
use webrtc::media::rtp::rtp_codec::RTPCodecType;
use webrtc::media::rtp::rtp_receiver::RTCRtpReceiver;
use webrtc::media::rtp::rtp_sender::RTCRtpSender;
use webrtc::media::rtp::RTCRtpCodingParameters;
use webrtc::media::rtp::RTCRtpReceiveParameters;
use webrtc::media::track::track_local::TrackLocal;
use webrtc::media::track::track_remote::TrackRemote;
use webrtc::peer::ice::ice_candidate::RTCIceCandidate;
use webrtc::peer::ice::ice_gather::ice_gatherer::RTCIceGatherer;
use webrtc::peer::ice::ice_gather::RTCIceGatherOptions;
use webrtc::peer::ice::ice_server::RTCIceServer;

use atomic::Atomic;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;

use tokio::sync::Mutex;

use crate::buffer::errors;

use super::errors::*;
use anyhow::Result;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const SIGNALER_LABEL: &'static str = "rsfu_relay_signaler";
const SIGNALER_REQUEST_EVENT: &'static str = "rsfu_relay_request";

//http://technosophos.com/2018/06/12/from-go-to-rust-json-and-yaml.html
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMeta {
    #[serde(rename = "streamId")]
    stream_id: String,
    #[serde(rename = "trackId")]
    track_id: String,
    //https://serde.rs/field-attrs.html#skip
    #[serde(skip_serializing, skip_deserializing)]
    codec_parameters: Option<RTCRtpCodecParameters>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    #[serde(rename = "encodings", skip_serializing_if = "Option::is_none")]
    encodings: Option<RTCRtpCodingParameters>,
    #[serde(rename = "iceCandidates")]
    ice_candidates: Vec<RTCIceCandidate>,
    #[serde(rename = "iceParameters")]
    ice_parameters: RTCIceParameters,
    #[serde(rename = "dtlsParameters")]
    dtls_parameters: DTLSParameters,
    #[serde(rename = "sctpTransportCapabilities")]
    sctp_capabilities: SCTPTransportCapabilities,
    #[serde(rename = "trackInfo", skip_serializing_if = "Option::is_none")]
    track_meta: Option<TrackMeta>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(rename = "id")]
    id: u64,
    #[serde(rename = "reply")]
    is_reply: bool,
    #[serde(rename = "event")]
    event: String,
    #[serde(rename = "payload")]
    payload: Vec<u8>,
}

pub struct Message {
    p: Option<Peer>,
    event: String,
    id: u64,
    msg: Vec<u8>,
}

pub struct PeerConfig {
    setting_engine: SettingEngine,
    ice_servers: Vec<RTCIceServer>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerMeta {
    #[serde(rename = "peerId")]
    peer_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
}

pub struct Options {
    // RelayMiddlewareDC if set to true middleware data channels will be created and forwarded
    // to the relayed peer
    relay_middleware_dc: bool,
    // RelaySessionDC if set to true fanout data channels will be created and forwarded to the
    // relayed peerF
    relay_session_dc: bool,
}

pub type OnPeerReadyFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;
pub type OnPeerCloseFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;
pub type OnPeerRequestFn = Box<
    dyn (FnMut(Arc<String>, Arc<Message>) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;
pub type OnPeerDataChannelFn =
    Box<dyn (FnMut(Arc<RTCDataChannel>) -> Pin<Box<dyn Future<Output = ()> + Send>>) + Send + Sync>;

pub type OnPeerTrackFn = Box<
    dyn (FnMut(
            Arc<TrackRemote>,
            Arc<RTCRtpReceiver>,
            TrackMeta,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;
#[derive(Clone)]
pub struct Peer {
    media_engine: Arc<Mutex<MediaEngine>>,
    api: Arc<API>,
    ice_transport: Arc<RTCIceTransport>,

    peer_meta: PeerMeta,
    sctp_transport: Arc<RTCSctpTransport>,
    dtls_transport: Arc<RTCDtlsTransport>,
    ice_role: Arc<RTCIceRole>,
    ready: bool,
    rtp_senders: Vec<Arc<RTCRtpSender>>,
    rtp_receivers: Vec<Arc<RTCRtpReceiver>>,
    pending_requests: HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>,
    local_tracks: Option<Arc<dyn TrackLocal + Send + Sync>>,
    signaling_dc: Arc<RTCDataChannel>,
    ice_gatherer: Arc<RTCIceGatherer>,
    dc_index: u16,

    on_ready_handler: Arc<Mutex<Option<OnPeerReadyFn>>>,
    on_close_handler: Arc<Mutex<Option<OnPeerCloseFn>>>,
    on_request_handler: Arc<Mutex<Option<OnPeerRequestFn>>>,
    on_data_channel_handler: Arc<Mutex<Option<OnPeerDataChannelFn>>>,
    on_track_handler: Arc<Mutex<Option<OnPeerTrackFn>>>,
}
use std::rc::Rc;
impl Peer {
    fn set_handlers(&mut self) {}
    async fn new(meta: PeerMeta, conf: PeerConfig) -> Result<Self> {
        let ice_options = RTCIceGatherOptions {
            ice_servers: conf.ice_servers,
            ..Default::default()
        };

        let me = MediaEngine::default();
        let api = APIBuilder::new()
            .with_media_engine(me)
            .with_setting_engine(conf.setting_engine)
            .build();

        // Create the ICE gatherer
        let gatherer = Arc::new(api.new_ice_gatherer(ice_options)?);
        // Construct the ICE transport
        let ice = Arc::new(api.new_ice_transport(Arc::clone(&gatherer)));
        // Construct the DTLS transport
        let dtls = Arc::new(api.new_dtls_transport(Arc::clone(&ice), vec![])?);
        // Construct the SCTP transport
        let sctp = Arc::new(api.new_sctp_transport(Arc::clone(&dtls))?);

        let mut p = Self {
            media_engine: Arc::new(Mutex::new(MediaEngine::default())),
            api: Arc::new(api),
            ice_transport: ice,
            peer_meta: meta,
            sctp_transport: Arc::clone(&sctp),
            dtls_transport: dtls,
            ice_role: Arc::new(RTCIceRole::default()),
            ready: false,
            rtp_senders: Vec::new(),
            rtp_receivers: Vec::new(),
            pending_requests: HashMap::new(),
            local_tracks: None,

            signaling_dc: Arc::new(RTCDataChannel::default()),
            ice_gatherer: gatherer,
            dc_index: 0,

            on_close_handler: Arc::new(Mutex::new(None)),
            on_data_channel_handler: Arc::new(Mutex::new(None)),
            on_ready_handler: Arc::new(Mutex::new(None)),
            on_track_handler: Arc::new(Mutex::new(None)),
            on_request_handler: Arc::new(Mutex::new(None)),
        };

        let mut arc_p = Arc::new(&p);

        let on_ready_handler = Arc::clone(&p.on_ready_handler);
        sctp.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
            if d.label() == SIGNALER_LABEL {
                arc_p.signaling_dc = Arc::clone(&d);
                let on_ready_handler2 = Arc::clone(&on_ready_handler);
                Box::pin(async move {
                    d.on_open(Box::new(move || {
                        let on_ready_handler3 = Arc::clone(&on_ready_handler2);
                        Box::pin(async move {
                            let mut handler = on_ready_handler3.lock().await;
                            if let Some(f) = &mut *handler {
                                f().await;
                            }
                        })
                    }))
                    .await;
                    // d.on_message(Box::new(move |d: DataChannelMessage| {
                    //     Box::pin(async move {
                    //         p.handle_request(d).await;
                    //     })
                    // }))
                    // .await;
                })
            } else {
                Box::pin(async {})
            }
        }))
        .await;

        Ok(p)
    }

    async fn handle_request(&mut self, msg: DataChannelMessage) -> Result<()> {
        let request: Request = serde_json::from_slice(&msg.data[..])?;

        let event = request.event;

        if event == SIGNALER_REQUEST_EVENT && !request.is_reply {
            let signal: Signal = serde_json::from_str(&event)?;
            self.receive(signal).await?;
            self.reply(request.id, event.clone(), &Vec::new()[..])
                .await?;
        }
        if request.is_reply {
            if let Some(chan) = self.pending_requests.get(&request.id) {
                chan.send(request.payload.clone())?;
                self.pending_requests.remove(&request.id);
            }
        }

        if event != SIGNALER_REQUEST_EVENT {
            let mut handler = self.on_request_handler.lock().await;
            if let Some(f) = &mut *handler {
                let message = Message {
                    p: Some(self.clone()),
                    event: event.clone(),
                    id: request.id,
                    msg: request.payload,
                };
                f(Arc::new(event), Arc::new(message)).await;
            }
        }

        Ok(())
    }

    async fn receive(&mut self, s: Signal) -> Result<()> {
        let codec_type: RTPCodecType;

        let codec_parameters = s.track_meta.clone().unwrap().codec_parameters.unwrap();
        if codec_parameters.capability.mime_type.starts_with("audio/") {
            codec_type = RTPCodecType::Audio;
        } else if codec_parameters.capability.mime_type.starts_with("video/") {
            codec_type = RTPCodecType::Video;
        } else {
            codec_type = RTPCodecType::Unspecified;
        }

        // let media_engine = Arc::clone(&self.media_engine);

        self.media_engine
            .try_lock()
            .unwrap()
            .register_codec(codec_parameters.to_owned(), codec_type)?;

        let rtp_receiver = self
            .api
            .new_rtp_receiver(codec_type, Arc::clone(&self.dtls_transport));

        let mut encodings = vec![];
        let coding_parameters = s.encodings.unwrap();
        if coding_parameters.ssrc != 0 {
            encodings.push(RTCRtpCodingParameters {
                ssrc: coding_parameters.ssrc,
                ..Default::default()
            });
        }
        encodings.push(RTCRtpCodingParameters {
            rid: coding_parameters.rid,
            ..Default::default()
        });

        rtp_receiver
            .receive(&RTCRtpReceiveParameters { encodings })
            .await?;

        rtp_receiver
            .set_rtp_parameters(RTCRtpParameters {
                header_extensions: Vec::new(),
                codecs: vec![codec_parameters],
            })
            .await;

        let arc_rtp_receiver = Arc::new(rtp_receiver);

        if let Some(track) = arc_rtp_receiver.track().await {
            let mut handler = self.on_track_handler.lock().await;
            if let Some(f) = &mut *handler {
                f(track, Arc::clone(&arc_rtp_receiver), s.track_meta.unwrap()).await;
            }
        }

        self.rtp_receivers.push(arc_rtp_receiver);

        Ok(())
    }

    pub async fn reply(&mut self, id: u64, event: String, payload: &[u8]) -> Result<()> {
        let req = Request {
            id,
            event,
            payload: Vec::from(payload),
            is_reply: true,
        };

        let req_json = serde_json::to_string(&req)?;
        self.signaling_dc.send(&Bytes::from(req_json)).await?;

        Ok(())
    }
}
