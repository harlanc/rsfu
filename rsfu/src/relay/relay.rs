use bytes::Bytes;
use rand::rngs::StdRng;
use rand::RngCore;
use rand::SeedableRng;
use rtcp::packet::Packet as RtcpPacket;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::api::APIBuilder;
use webrtc::api::API;

use interceptor::noop::NoOp;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::data_channel_parameters::DataChannelParameters;
use webrtc::data_channel::RTCDataChannel;
use webrtc::dtls_transport::dtls_parameters::DTLSParameters;
use webrtc::dtls_transport::RTCDtlsTransport;
use webrtc::ice_transport::ice_parameters::RTCIceParameters;
use webrtc::ice_transport::ice_role::RTCIceRole;
use webrtc::ice_transport::RTCIceTransport;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpParameters;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;
use webrtc::rtp_transceiver::RTCRtpCodingParameters;
use webrtc::rtp_transceiver::RTCRtpReceiveParameters;
use webrtc::rtp_transceiver::RTCRtpSendParameters;
use webrtc::sctp_transport::sctp_transport_capabilities::SCTPTransportCapabilities;
use webrtc::sctp_transport::RTCSctpTransport;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_remote::TrackRemote;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::Duration;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::ice_transport::ice_gatherer::RTCIceGatherOptions;
use webrtc::ice_transport::ice_gatherer::RTCIceGatherer;
use webrtc::ice_transport::ice_gatherer_state::RTCIceGathererState;
use webrtc::ice_transport::ice_server::RTCIceServer;

use super::errors::*;
use anyhow::Result;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
const SIGNALER_LABEL: &'static str = "rsfu_relay_signaler";
const SIGNALER_REQUEST_EVENT: &'static str = "rsfu_relay_request";

const SEED: [u8; 32] = [
    1, 0, 0, 0, 23, 0, 0, 0, 200, 1, 0, 0, 210, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0,
];

//http://technosophos.com/2018/06/12/from-go-to-rust-json-and-yaml.html
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMeta {
    #[serde(rename = "streamId")]
    stream_id: String,
    #[serde(rename = "trackId")]
    track_id: String,
    //https://serde.rs/field-attrs.html#skip
    #[allow(dead_code)]
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

#[allow(dead_code)]
pub struct Message {
    p: Option<Peer>,
    event: String,
    id: u64,
    msg: Vec<u8>,
}

impl Message {
    #[allow(dead_code)]
    fn payload(&self) -> Vec<u8> {
        self.msg.clone()
    }
    #[allow(dead_code)]
    async fn reply(self, msg: Vec<u8>) -> Result<()> {
        self.p.unwrap().reply(self.id, self.event, &msg).await
    }
}

pub struct PeerConfig {
    pub setting_engine: SettingEngine,
    pub ice_servers: Vec<RTCIceServer>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerMeta {
    #[serde(rename = "peerId")]
    pub peer_id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
}
#[allow(dead_code)]
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
    #[allow(dead_code)]
    peer_meta: PeerMeta,
    sctp_transport: Arc<RTCSctpTransport>,
    dtls_transport: Arc<RTCDtlsTransport>,
    ice_role: Arc<RTCIceRole>,
    ready: bool,
    rtp_senders: Vec<Arc<RTCRtpSender>>,
    rtp_receivers: Vec<Arc<RTCRtpReceiver>>,
    pending_requests: Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>,
    local_tracks: Vec<Arc<dyn TrackLocal + Send + Sync>>,
    signaling_dc: Arc<RTCDataChannel>,
    ice_gatherer: Arc<RTCIceGatherer>,
    dc_index: u16,

    on_ready_handler: Arc<Mutex<Option<OnPeerReadyFn>>>,
    on_close_handler: Arc<Mutex<Option<OnPeerCloseFn>>>,
    on_request_handler: Arc<Mutex<Option<OnPeerRequestFn>>>,
    on_data_channel_handler: Arc<Mutex<Option<OnPeerDataChannelFn>>>,
    on_track_handler: Arc<Mutex<Option<OnPeerTrackFn>>>,
    #[allow(dead_code)]
    on_data_channel_callback_handler: Arc<Mutex<fn(&RTCDataChannel)>>,
}

fn on_data_channel_callback(_channel: &RTCDataChannel) {}
impl Peer {
    pub(crate) fn new(meta: PeerMeta, conf: PeerConfig) -> Result<Self> {
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

        let signaling_dc = Arc::new(RTCDataChannel::default());

        let p = Self {
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
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            local_tracks: Vec::new(),

            signaling_dc: Arc::clone(&signaling_dc),
            ice_gatherer: gatherer,
            dc_index: 0,

            on_close_handler: Arc::new(Mutex::new(None)),
            on_data_channel_handler: Arc::new(Mutex::new(None)),
            on_ready_handler: Arc::new(Mutex::new(None)),
            on_track_handler: Arc::new(Mutex::new(None)),
            on_request_handler: Arc::new(Mutex::new(None)),

            on_data_channel_callback_handler: Arc::new(Mutex::new(on_data_channel_callback)),
        };

        Ok(p)
    }

    #[allow(dead_code)]
    async fn init(&mut self, meta: PeerMeta, conf: PeerConfig) {
        let on_ready_handler = Arc::clone(&self.on_ready_handler);

        self.sctp_transport
            .on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
                if d.label() == SIGNALER_LABEL {
                    let on_ready_handler2 = Arc::clone(&on_ready_handler);
                    Box::pin(async move {
                        d.on_open(Box::new(move || {
                            Box::pin(async move {
                                let mut handler = on_ready_handler2.lock().await;
                                if let Some(f) = &mut *handler {
                                    f().await;
                                }
                            })
                        }));
                        d.on_message(Box::new(move |d: DataChannelMessage| {
                            Box::pin(async move {
                                //  self.handle_request(d).await;
                            })
                        }));
                    })
                } else {
                    Box::pin(async {})
                }
            }));
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
            if let Some(chan) = self.pending_requests.lock().await.get(&request.id) {
                chan.send(request.payload.clone())?;
                self.pending_requests.lock().await.remove(&request.id);
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

    fn id(&self) -> String {
        self.peer_meta.peer_id.clone()
    }
    // Offer is used for establish the connection of the local relay Peer
    // with the remote relay Peer.
    // If connection is successful OnReady handler will be called
    async fn offer(
        &mut self,
        signalFn: fn(meta: PeerMeta, singal: &str) -> Result<String>,
    ) -> Result<()> {
        if self.ice_gatherer.state() != RTCIceGathererState::New {
            return Err(Error::ErrRelayPeerSignalDone.into());
        }

        let (gather_finished_tx, mut gather_finished_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut gather_finished_tx = Some(gather_finished_tx);
        self.ice_gatherer
            .on_local_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
                if c.is_none() {
                    gather_finished_tx.take();
                }
                Box::pin(async {})
            }));

        self.ice_gatherer.gather().await?;

        let _ = gather_finished_rx.recv().await;

        let ice_candidates = self.ice_gatherer.get_local_candidates().await?;

        let ice_parameters = self.ice_gatherer.get_local_parameters().await?;

        let dtls_parameters = self.dtls_transport.get_local_parameters()?;

        let sctp_capabilities = self.sctp_transport.get_capabilities();

        let local_signal = Signal {
            ice_candidates,
            ice_parameters,
            dtls_parameters,
            sctp_capabilities,

            encodings: None,
            track_meta: None,
        };

        self.ice_role = Arc::new(RTCIceRole::Controlling);
        let json_str = serde_json::to_string(&local_signal)?;

        let data = signalFn(self.peer_meta.clone(), &json_str)?;

        let remote_signal = serde_json::from_str::<Signal>(&data)?;

        self.start(remote_signal).await?;

        self.signaling_dc = Arc::new(self.create_data_channel(SIGNALER_LABEL.to_string()).await?);

        let on_ready_handler2 = Arc::clone(&self.on_ready_handler);

        self.signaling_dc.on_open(Box::new(move || {
            Box::pin(async move {
                let mut handler = on_ready_handler2.lock().await;
                if let Some(f) = &mut *handler {
                    f().await;
                }
            })
        }));

        //self.signaling_dc.on_message(f)

        Ok(())
    }

    pub async fn on_close(&self, f: OnPeerCloseFn) {
        let mut handler = self.on_close_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn answer(&mut self, request: String) -> Result<String> {
        if self.ice_gatherer.state() != RTCIceGathererState::New {
            return Err(Error::ErrRelayPeerSignalDone.into());
        }
        let (gather_finished_tx, mut gather_finished_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut gather_finished_tx = Some(gather_finished_tx);
        self.ice_gatherer
            .on_local_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
                if c.is_none() {
                    gather_finished_tx.take();
                }
                Box::pin(async {})
            }));

        self.ice_gatherer.gather().await?;

        let _ = gather_finished_rx.recv().await;

        let ice_candidates = self.ice_gatherer.get_local_candidates().await?;
        let ice_parameters = self.ice_gatherer.get_local_parameters().await?;
        let dtls_parameters = self.dtls_transport.get_local_parameters()?;
        let sctp_capabilities = self.sctp_transport.get_capabilities();

        let signal = Signal {
            ice_candidates,
            ice_parameters,
            dtls_parameters,
            sctp_capabilities,
            encodings: None,
            track_meta: None,
        };

        self.ice_role = Arc::new(RTCIceRole::Controlled);

        let signal_2 = serde_json::from_str::<Signal>(&request)?;

        self.start(signal_2).await?;

        let json_str = serde_json::to_string(&signal)?;

        Ok(json_str)
    }

    pub async fn write_rtcp(&self, pkt: &[Box<dyn RtcpPacket + Send + Sync>]) -> Result<()> {
        self.dtls_transport.write_rtcp(pkt).await?;
        Ok(())
    }

    pub fn get_local_tracks(&self) -> Vec<Arc<dyn TrackLocal + Send + Sync>> {
        return self.local_tracks.clone();
    }

    pub async fn on_ready(&self, f: OnPeerReadyFn) {
        let mut handler = self.on_ready_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_data_channel(&self, f: OnPeerDataChannelFn) {
        let mut handler = self.on_data_channel_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_track(&self, f: OnPeerTrackFn) {
        let mut handler = self.on_track_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn on_request(&self, f: OnPeerRequestFn) {
        let mut handler = self.on_request_handler.lock().await;
        *handler = Some(f);
    }

    pub async fn create_data_channel(&mut self, label: String) -> Result<RTCDataChannel> {
        let idx = self.dc_index;

        self.dc_index += 1;
        let dc_parameters = DataChannelParameters {
            label,
            ordered: true,
            ..Default::default()
        };

        let rv = self
            .api
            .new_data_channel(Arc::clone(&self.sctp_transport), dc_parameters)
            .await?;

        Ok(rv)
    }

    async fn start(&mut self, sig: Signal) -> Result<()> {
        self.ice_transport
            .set_remote_candidates(&sig.ice_candidates)
            .await?;

        // Start the ICE transport
        self.ice_transport
            .start(&sig.ice_parameters, Some(*self.ice_role))
            .await?;

        // Start the DTLS transport
        self.dtls_transport
            .start(sig.dtls_parameters.clone())
            .await?;

        // Start the SCTP transport
        self.sctp_transport.start(sig.sctp_capabilities).await?;

        self.ready = true;
        Ok(())
    }

    pub async fn close(&mut self) -> Vec<Result<()>> {
        let mut results: Vec<Result<()>> = Vec::new();
        for sender in &self.rtp_senders {
            match sender.stop().await {
                Err(err) => {
                    results.push(Err(err.into()));
                }
                Ok(_) => results.push(Ok(())),
            }
        }

        for receiver in &self.rtp_receivers {
            match receiver.stop().await {
                Err(err) => {
                    results.push(Err(err.into()));
                }
                Ok(_) => results.push(Ok(())),
            }
        }

        match self.sctp_transport.stop().await {
            Err(err) => {
                results.push(Err(err.into()));
            }
            Ok(_) => results.push(Ok(())),
        }

        match self.dtls_transport.stop().await {
            Err(err) => {
                results.push(Err(err.into()));
            }
            Ok(_) => results.push(Ok(())),
        }

        match self.ice_transport.stop().await {
            Err(err) => {
                results.push(Err(err.into()));
            }
            Ok(_) => results.push(Ok(())),
        }

        let mut handler = self.on_close_handler.lock().await;
        if let Some(f) = &mut *handler {
            f().await;
        }

        results
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

        let rtp_receiver = self.api.new_rtp_receiver(
            codec_type,
            Arc::clone(&self.dtls_transport),
            Arc::new(NoOp {}),
        );

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

    pub async fn add_track(
        &mut self,
        receiver: Arc<RTCRtpReceiver>,
        remote_track: Arc<TrackRemote>,
        local_track: Arc<dyn TrackLocal + Send + Sync>,
    ) -> Result<Arc<RTCRtpSender>> {
        let codec = remote_track.codec().await;

        let sdr = self
            .api
            .new_rtp_sender(
                Some(Arc::clone(&local_track)),
                Arc::clone(&self.dtls_transport),
                Arc::new(NoOp {}),
            )
            .await;

        self.media_engine
            .lock()
            .await
            .register_codec(codec.clone(), remote_track.kind())?;

        let track_meta = TrackMeta {
            stream_id: remote_track.stream_id().await,
            track_id: remote_track.id().await,
            codec_parameters: Some(codec),
        };

        let encodings = RTCRtpCodingParameters {
            ssrc: sdr.get_parameters().await.encodings[0].ssrc,
            payload_type: remote_track.payload_type(),
            ..Default::default()
        };

        let signal = Signal {
            encodings: Some(encodings.clone()),
            ice_candidates: Vec::new(),
            ice_parameters: RTCIceParameters::default(),
            dtls_parameters: DTLSParameters::default(),
            sctp_capabilities: SCTPTransportCapabilities {
                max_message_size: 0,
            },
            track_meta: Some(track_meta),
        };

        let payload = serde_json::to_string(&signal)?;

        let timeout_duration = Duration::from_secs(2);

        self.request(
            timeout_duration,
            SIGNALER_REQUEST_EVENT.to_string(),
            payload.into_bytes(),
        )
        .await?;

        let parameters = receiver.get_parameters().await.clone();

        let send_parameters = RTCRtpSendParameters {
            rtp_parameters: parameters.clone(),
            encodings: vec![RTCRtpCodingParameters {
                ssrc: encodings.ssrc,
                payload_type: encodings.payload_type,
                ..Default::default()
            }],
        };

        sdr.send(&send_parameters).await?;

        self.local_tracks.push(local_track);

        let sdr_arc = Arc::new(sdr);

        self.rtp_senders.push(sdr_arc.clone());

        Ok(sdr_arc)
    }

    async fn emit(&mut self, event: String, payload: Vec<u8>) -> Result<()> {
        let mut rng0 = StdRng::from_seed(SEED);

        let req = Request {
            id: rng0.next_u64(),
            is_reply: false,
            event,
            payload,
        };

        let json_str = serde_json::to_string(&req)?;

        self.signaling_dc.send(&Bytes::from(json_str)).await?;

        Ok(())
    }

    async fn request(
        &mut self,
        timeout: Duration,
        event: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let mut rng0 = StdRng::from_seed(SEED);
        let req = Request {
            id: rng0.next_u64(),
            is_reply: false,
            event,
            payload,
        };

        let json_str = serde_json::to_string(&req)?;

        self.signaling_dc.send(&Bytes::from(json_str)).await?;

        let timer = tokio::time::sleep(timeout);
        tokio::pin!(timer);

        let (event_publisher, mut event_consumer) = mpsc::unbounded_channel();
        self.pending_requests
            .lock()
            .await
            .insert(req.id, event_publisher); //[&req.id] = event_publisher;

        tokio::select! {
            _ = timer.as_mut() =>{
                self.pending_requests.lock().await.remove(&req.id);
                return Err(Error::ErrRelayRequestTimeout.into());
            },
            data = event_consumer.recv() => {
                self.pending_requests.lock().await.remove(&req.id);

                if let Some(payload)  = data{
                    return Ok(payload);
                }else{
                    return Err(Error::ErrRelayRequestEmptyRespose.into());
                }
            },
        };
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
