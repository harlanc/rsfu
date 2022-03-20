use super::router::Router;
use super::router::RouterLocal;
use super::session::Session;
use super::sfu::WebRTCTransportConfig;
use rtcp::packet::Packet as RtcpPacket;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
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
struct Publisher {
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
    data_channels: Vec<RTCDataChannel>,

    with_sr_reports: bool,
    relay_fanout_data_channels: bool,
}

struct TestA {}

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
            factory: AtomicFactory::new(1000, 1000),
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
        let session_out = Arc::clone(&mut self.session);
        let tracks_out = Arc::clone(&mut self.tracks);
        let relay_peer_out = Arc::clone(&mut self.relay_peers);
        self.pc
            .on_track(Box::new(
                move |track: Option<Arc<TrackRemote>>, receiver: Option<Arc<RTCRtpReceiver>>| {
                    let router_in = Arc::clone(&router_out);
                    let router_in2 = Arc::clone(&router_out);
                    let session_in = Arc::clone(&session_out);
                    let tracks_in = Arc::clone(&tracks_out);
                    let relay_peers_in = Arc::clone(&relay_peer_out);
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
                                        track_val,
                                        track_id,
                                        track_stream_id,
                                    )
                                    .await;

                                let receiver_clone = receiver.clone();

                                if publish {
                                    session_in.lock().await.publish(router_in2, receiver);

                                    tracks_in.lock().await.push(PublisherTrack {
                                        track: track_val_clone,
                                        receiver: receiver_clone,
                                        client_relay: true,
                                    });

                                    let relay_peers = relay_peers_in.lock().await;

                                    for val in &*relay_peers {
                                        //val.
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
    }

    async fn crate_relay_track(
        &mut self,
        track: TrackRemote,
        receiver: Arc<Mutex<dyn Receiver + Send + Sync>>,
        rp: &mut Peer,
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

        let mut downtrack = DownTrack::new(
            c,
            receiver.clone(),
            self.id.clone(),
            self.cfg.Router.max_packet_track,
        );

        let downtrack_arc = Arc::new(downtrack);

        let receiver_mg = receiver.lock().await;
        let media_ssrc = track.ssrc();
        if let Some(webrtc_receiver) = (*receiver_mg).as_any().downcast_ref::<WebRTCReceiver>() {
            let sdr = rp
                .add_track(webrtc_receiver.receiver.clone(), track, downtrack_arc.clone())
                .await?;

            let ssrc = sdr.get_parameters().await.encodings.get(0).unwrap().ssrc;

            let rtcp_buffer = self.cfg.factory.get_or_new_rtcp_buffer(ssrc).await;
            let pc_out = self.pc.clone();
            rtcp_buffer
                .lock()
                .await
                .on_packet(Box::new(move |bytes: Vec<u8>| {
                    let pc_in = pc_out.clone();
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
            downtrack_arc.on_close_hander(Box::new(move || {
                let sdr_in = sdr_out.clone();
                Box::pin(async move { if let Err(_) = sdr_in.stop().await {} })
            }));
        }

        Ok(())
    }
}
