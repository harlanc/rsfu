use super::data_channel::DataChannel;
use super::router::Router;
use super::router::RouterLocal;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use crate::buffer::rtcpreader::RTCPReader;
use rtcp::packet::Packet as RtcpPacket;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_gatherer::OnLocalCandidateHdlrFn;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::signaling_state::RTCSignalingState;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::rtp_transceiver::RTCPFeedback;

use webrtc::peer_connection::configuration::RTCConfiguration;

use super::receiver::WebRTCReceiver;

use super::media_engine;
use super::receiver::Receiver;
use crate::relay::relay::Peer;
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Once;
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::{sleep, Duration};
use webrtc::api::APIBuilder;
use webrtc::api::API;
use webrtc::data_channel::RTCDataChannel;
use webrtc::track::track_remote::TrackRemote;

use super::down_track::DownTrack;
use crate::buffer::factory::AtomicFactory;

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

pub struct Publisher {
    id: String,

    pc: Arc<RTCPeerConnection>,
    cfg: WebRTCTransportConfig,

    router: Arc<Mutex<dyn Router + Send + Sync>>,
    session: Arc<Mutex<dyn Session + Send + Sync>>,
    tracks: Arc<Mutex<Vec<PublisherTrack>>>,

    relayed: AtomicBool,
    relay_peers: Arc<Mutex<Vec<RelayPeer>>>,
    candidates: Vec<RTCIceCandidateInit>,

    on_ice_connection_state_change_hander: Arc<Mutex<Option<OnIceConnectionStateChange>>>,
    on_publisher_track: Arc<Mutex<Option<OnPublisherTrack>>>,

    close_once: Once,
}

pub struct RelayPeer {
    peer: Peer,
    data_channels: Vec<Arc<RTCDataChannel>>,

    with_sr_reports: bool,
    relay_fanout_data_channels: bool,
}

struct TestA {}

#[derive(Clone)]
pub(super) struct PublisherTrack {
    track: Arc<TrackRemote>,
    receiver: Arc<Mutex<dyn Receiver + Send + Sync>>,

    // This will be used in the future for tracks that will be relayed as clients or servers
    // This is for SVC and Simulcast where you will be able to chose if the relayed peer just
    // want a single track (for recording/ processing) or get all the tracks (for load balancing)
    client_relay: bool,
}

impl Publisher {
    pub async fn new(
        id: String,
        session: Arc<Mutex<dyn Session + Send + Sync>>,
        cfg: WebRTCTransportConfig,
    ) -> Result<Self> {
        let me = media_engine::get_publisher_media_engine().await?;

        let setting_engine = cfg.setting.clone();

        let api = APIBuilder::new()
            .with_media_engine(me)
            .with_setting_engine(setting_engine)
            .build();

        let router = cfg.Router.clone();

        let rtc_config_clone = RTCConfiguration {
            ice_servers: cfg.configuration.ice_servers.clone(),
            ..Default::default()
        };

        let config_clone = WebRTCTransportConfig {
            configuration: rtc_config_clone,
            setting: cfg.setting.clone(),
            Router: cfg.Router.clone(),
            factory: Arc::new(Mutex::new(AtomicFactory::new(1000, 1000))),
        };

        let pc = api.new_peer_connection(cfg.configuration).await?;

        let mut publisher = Publisher {
            id: id.clone(),
            pc: Arc::new(pc),
            cfg: config_clone,
            router: Arc::new(Mutex::new(RouterLocal::new(id, session.clone(), router))),
            session: session,

            tracks: Arc::new(Mutex::new(Vec::new())),

            relayed: AtomicBool::new(false),
            relay_peers: Arc::new(Mutex::new(Vec::new())),
            candidates: Vec::new(),
            on_ice_connection_state_change_hander: Arc::default(),
            on_publisher_track: Arc::default(),
            close_once: Once::new(),
        };

        publisher.on_track().await;

        Ok(publisher)
    }

    async fn on_track(&mut self) {
        let router_out = Arc::clone(&mut self.router);
        let router_out_2 = Arc::clone(&mut self.router);
        let session_out = Arc::clone(&mut self.session);
        let session_out_2 = Arc::clone(&mut self.session);
        let tracks_out = Arc::clone(&mut self.tracks);
        let relay_peer_out = Arc::clone(&mut self.relay_peers);
        let relay_peer_out_2 = Arc::clone(&mut self.relay_peers);
        let factory_out = Arc::clone(&mut self.cfg.factory);
        let peer_id_out = self.id.clone();
        let peer_id_out_2 = self.id.clone();
        let max_packet_track = self.cfg.Router.max_packet_track;
        let peer_connection_out = self.pc.clone();
        let peer_connection_out_2 = self.pc.clone();

        self.pc
            .on_track(Box::new(
                move |track: Option<Arc<TrackRemote>>, receiver: Option<Arc<RTCRtpReceiver>>| {
                    let router_in = Arc::clone(&router_out);
                    let router_in2 = Arc::clone(&router_out);
                    let session_in = Arc::clone(&session_out);
                    let tracks_in = Arc::clone(&tracks_out);
                    let relay_peers_in = Arc::clone(&relay_peer_out);
                    let factory_in = Arc::clone(&factory_out);
                    let peer_id = peer_id_out.clone();
                    let peer_connection = peer_connection_out.clone();

                    Box::pin(async move {
                        if let Some(receiver_val) = receiver {
                            if let Some(track_val) = track {
                                let mut router = router_in.lock().await;
                                let track_id = track_val.id().await;
                                let track_stream_id = track_val.stream_id().await;
                                let track_val_clone = track_val.clone();

                                let (receiver, publish) = router
                                    .add_receiver(
                                        receiver_val,
                                        track_val.clone(),
                                        track_id,
                                        track_stream_id,
                                    )
                                    .await;

                                let receiver_clone = receiver.clone();

                                if publish {
                                    session_in
                                        .lock()
                                        .await
                                        .publish(router_in2, receiver.clone());

                                    tracks_in.lock().await.push(PublisherTrack {
                                        track: track_val_clone,
                                        receiver: receiver_clone,
                                        client_relay: true,
                                    });

                                    let mut relay_peers = relay_peers_in.lock().await;

                                    for val in &mut *relay_peers {
                                        Publisher::crate_relay_track(
                                            track_val.clone(),
                                            receiver.clone(),
                                            &mut val.peer,
                                            peer_id.clone(),
                                            max_packet_track,
                                            factory_in.clone(),
                                            peer_connection.clone(),
                                        );
                                    }
                                } else {
                                    tracks_in.lock().await.push(PublisherTrack {
                                        track: track_val_clone,
                                        receiver,
                                        client_relay: false,
                                    })
                                }
                            }
                        }
                    })
                },
            ))
            .await;

        self.pc
            .on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
                let session_in = Arc::clone(&session_out_2);
                let peer_id = peer_id_out_2.clone();

                // Ignore our default channel, exists to force ICE candidates. See signalPair for more info
                if d.label() == super::subscriber::API_CHANNEL_LABEL {
                    return Box::pin(async {});
                }

                Box::pin(async move {
                    session_in.lock().await.add_data_channel(peer_id, d);
                })
            }))
            .await;

        self.pc
            .on_ice_connection_state_change(Box::new(move |s: RTCIceConnectionState| {
                let router_in = Arc::clone(&router_out_2);
                let relay_peer_in = Arc::clone(&relay_peer_out_2);

                let peer_connection_in = peer_connection_out_2.clone();
                Box::pin(async move {
                    match s {
                        RTCIceConnectionState::Failed | RTCIceConnectionState::Closed => {
                            Publisher::close(relay_peer_in, router_in, peer_connection_in).await;
                        }

                        _ => {}
                    }
                })
            }))
            .await;

        self.router
            .lock()
            .await
            .set_peer_connection(self.pc.clone());
    }

    async fn answer(&self, offer: RTCSessionDescription) -> Result<RTCSessionDescription> {
        self.pc.set_remote_description(offer).await?;

        for c in &self.candidates {
            if let Err(err) = self.pc.add_ice_candidate(c.clone()).await {}
        }

        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer.clone()).await?;

        Ok(answer)
    }

    fn get_router(&self) -> Arc<Mutex<dyn Router + Send + Sync>> {
        self.router.clone()
    }

    async fn on_publisher_track_handler(&mut self, f: OnPublisherTrack) {
        let mut handler = self.on_publisher_track.lock().await;
        *handler = Some(f);
    }

    async fn on_ice_candidate_handler(&mut self, f: OnLocalCandidateHdlrFn) {
        self.pc.on_ice_candidate(f).await;
    }

    async fn on_ice_connection_state_change_handler(&mut self, f: OnIceConnectionStateChange) {
        let mut handler = self.on_ice_connection_state_change_hander.lock().await;
        *handler = Some(f);
    }

    fn signaling_state(&self) -> RTCSignalingState {
        self.pc.signaling_state()
    }

    fn peer_connection(&self) -> Arc<RTCPeerConnection> {
        self.pc.clone()
    }

    //fn relay()

    async fn publisher_tracks(&self) -> Vec<PublisherTrack> {
        self.tracks.lock().await.clone()
    }

    async fn add_relay_fanout_data_channel(&mut self, label: &String) {
        for rp in &mut *self.relay_peers.lock().await {
            for dc in &rp.data_channels {
                if dc.label() == label {
                    continue;
                }
            }

            let rv = rp.peer.create_data_channel(label.clone()).await;

            if let Ok(dc) = rv {
                let label_out = label.clone();
                let session_out = Arc::clone(&mut self.session);
                dc.on_message(Box::new(move |msg: DataChannelMessage| {
                    let session_in = Arc::clone(&session_out);
                    let label_in = label_out.clone();
                    Box::pin(async move {
                        session_in
                            .lock()
                            .await
                            .fanout_message(String::from(""), label_in, msg)
                    })
                }))
                .await;
            }
        }
    }

    async fn get_relayed_data_channels(&self, label: String) -> Vec<Arc<RTCDataChannel>> {
        let mut data_channels = Vec::new();

        for rp in &mut *self.relay_peers.lock().await {
            for dc in &rp.data_channels {
                if dc.label() == label {
                    data_channels.push(dc.clone());
                }
            }
        }
        data_channels
    }

    fn relayed(&self) -> bool {
        self.relayed.load(Ordering::Relaxed)
    }

    async fn tracks(&self) -> Vec<Arc<TrackRemote>> {
        let mut tracks = Vec::new();

        for publisher_track in &*self.tracks.lock().await {
            tracks.push(publisher_track.track.clone())
        }

        tracks
    }

    async fn add_ice_candidata(&mut self, candidate: RTCIceCandidateInit) -> Result<()> {
        if let Some(desp) = self.pc.remote_description().await {
            self.pc.add_ice_candidate(candidate.clone()).await?;
        }

        self.candidates.push(candidate.clone());

        Ok(())
    }

    // fn crate_relay_track(
    //     &mut self,
    //     track: TrackRemote,
    //     receiver: Receiver,
    //     rp: Peer,
    // ) -> Result<()> {

    //     Ok(())
    // }

    async fn crate_relay_track(
        // &mut self,
        track: Arc<TrackRemote>,
        receiver: Arc<Mutex<dyn Receiver + Send + Sync>>,
        rp: &mut Peer,
        peer_id: String,
        max_packet_track: i32,
        //rtcp_reader: Arc<Mutex<RTCPReader>>,
        factory: Arc<Mutex<AtomicFactory>>,
        rtc_peer_connection: Arc<RTCPeerConnection>,
    ) -> Result<()> {
        let codec = track.codec().await;

        let c = RTCRtpCodecCapability {
            mime_type: codec.capability.mime_type,
            clock_rate: codec.capability.clock_rate,
            channels: codec.capability.channels,
            sdp_fmtp_line: codec.capability.sdp_fmtp_line,
            rtcp_feedback: vec![
                RTCPFeedback {
                    typ: String::from("nack"),
                    parameter: String::from(""),
                },
                RTCPFeedback {
                    typ: String::from("nack"),
                    parameter: String::from("pli"),
                },
            ],
        };

        let mut downtrack = DownTrack::new(c, receiver.clone(), peer_id, max_packet_track);

        let downtrack_arc = Arc::new(downtrack);

        let mut receiver_mg = receiver.lock().await;
        let media_ssrc = track.ssrc();
        if let Some(webrtc_receiver) = (receiver_mg).as_any().downcast_ref::<WebRTCReceiver>() {
            // let down_track = downtrack_arc.lock().await;
            let sdr = rp
                .add_track(
                    webrtc_receiver.receiver.clone(),
                    track,
                    downtrack_arc.clone(),
                )
                .await?;

            let ssrc = sdr.get_parameters().await.encodings.get(0).unwrap().ssrc;

            let rtcp_reader = factory.lock().await.get_or_new_rtcp_buffer(ssrc).await;
            // let pc_out = self.pc.clone();
            rtcp_reader
                .lock()
                .await
                .on_packet(Box::new(move |bytes: Vec<u8>| {
                    let pc_in = rtc_peer_connection.clone();
                    Box::pin(async move {
                        let mut buf = &bytes[..];
                        let mut pkts_result = rtcp::packet::unmarshal(&mut buf);
                        let mut pkts;

                        match pkts_result {
                            Ok(pkts_rv) => {
                                pkts = pkts_rv;
                            }
                            Err(_) => {
                                return;
                            }
                        }
                        let mut rpkts: Vec<Box<dyn RtcpPacket + Send + Sync>> = Vec::new();
                        for pkt in &mut pkts {
                            if let Some(pic_loss_indication) = pkt
                                    .as_any()
                                    .downcast_ref::<rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication>()
                                {
                                    let mut pli = pic_loss_indication.clone();
                                    pli.media_ssrc = media_ssrc;
                                    rpkts.push(Box::new(pli));

                                }
                        }
                        if rpkts.len() > 0 {
                            match pc_in.write_rtcp(&pkts[..]).await {
                                Ok(_) => {}
                                Err(_) => {}
                            }
                        }
                    })
                }))
                .await;

            let sdr_out = sdr.clone();

            // downtrack_arc.on_close_hander(Box::new(move || {
            //     let sdr_in = sdr_out.clone();
            //     Box::pin(async move { if let Err(_) = sdr_in.stop().await {} })
            // }));

            receiver_mg.add_down_track(downtrack_arc, true);
        }

        Ok(())
    }

    async fn close(
        relay_peers: Arc<Mutex<Vec<RelayPeer>>>,
        router: Arc<Mutex<dyn Router + Send + Sync>>,
        pc: Arc<RTCPeerConnection>,
    ) {
        // self.close_once.call_once(|| {
        //     Box::pin(async move {
        let mut peers = relay_peers.lock().await;

        for val in &mut *peers {
            val.peer.close().await;
        }
        router.lock().await.stop().await;
        pc.close().await;
        //     });
        // });
    }

    async fn relay_reports(&self, rp: &mut Peer) {
        loop {
            sleep(Duration::from_secs(5)).await;

            let local_tracks = rp.get_local_tracks();

            let mut rtcp_packets: Vec<Box<(dyn rtcp::packet::Packet + Send + Sync + 'static)>> =
                vec![];
            for local_track in local_tracks {
                if let Some(down_track) = local_track
                    .as_any()
                    .downcast_ref::<super::down_track::DownTrack>()
                {
                    if !down_track.bound.load(Ordering::Relaxed) {
                        continue;
                    }

                    if let Some(sr) = down_track.create_sender_report().await {
                        rtcp_packets.push(Box::new(sr));
                    }
                }
            }

            if rtcp_packets.len() == 0 {
                continue;
            }

            match rp.write_rtcp(&rtcp_packets[..]).await {
                Ok(_) => {}
                Err(err) => {}
            }
            // if self.closed {
            //     return Err(Error::ErrIOEof.into());
            // }

            // if self.ext_packets.len() > 0 {
            //     let ext_pkt = self.ext_packets.pop_front().unwrap();
            //     return Ok(ext_pkt);
            // }
        }
    }
}
