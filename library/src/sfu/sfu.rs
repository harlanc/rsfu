use super::router::RouterConfig;

use super::turn::TurnConfig;
use std::net::SocketAddr;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::peer_connection::configuration::RTCConfiguration;

use super::data_channel::DataChannel;
use super::session::Session;
use anyhow::Result;
use bytes::BytesMut;
use std::collections::HashMap;
use std::time::Duration;
use tokio::net::UdpSocket;
use turn::server::Server as TurnServer;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;

use std::sync::Arc;
use turn::auth::AuthHandler;
use webrtc::peer_connection::configuration::*;
use webrtc::peer_connection::policy::sdp_semantics::RTCSdpSemantics;
use webrtc_ice::mdns::MulticastDnsMode;
use webrtc_ice::udp_mux::*;
use webrtc_ice::udp_network::*;
#[derive(Clone)]
struct ICEServerConfig {
    urls: Vec<String>,
    user_name: String,
    credential: String,
}
#[derive(Clone)]
struct Candidates {
    ice_lite: bool,
    nat1_to_1ips: Vec<String>,
}
#[derive(Default)]
pub struct WebRTCTransportConfig {
    pub configuration: RTCConfiguration,
    pub setting: SettingEngine,
    Router: RouterConfig,
}
#[derive(Clone)]
struct WebRTCTimeoutsConfig {
    ice_disconnected_timeout: i32,
    ice_failed_timeout: i32,
    ice_keepalive_interval: i32,
}
#[derive(Clone)]
struct WebRTCConfig {
    ice_single_port: i32,
    ice_port_range: Vec<u16>,
    ice_servers: Vec<ICEServerConfig>,
    candidates: Candidates,
    sdp_semantics: String,
    mdns: bool,
    timeouts: WebRTCTimeoutsConfig,
}
#[derive(Clone)]
struct SFUConfig {
    ballast: i64,
    with_stats: bool,
}
#[derive(Clone)]
struct Config {
    sfu: SFUConfig,
    webrtc: WebRTCConfig,
    router: RouterConfig,
    turn: TurnConfig,
    turn_auth: Option<Arc<dyn AuthHandler + Send + Sync>>,
}

#[derive(Default)]
struct SFU {
    webrtc: WebRTCTransportConfig,
    turn: Option<TurnServer>,
    sessions: HashMap<String, Box<dyn Session>>,
    data_channels: Vec<DataChannel>,
    with_status: bool,
}

impl WebRTCTransportConfig {
    async fn new(c: &Config) -> Result<Self> {
        let mut se = SettingEngine::default();
        se.disable_media_engine_copy(true);

        if c.webrtc.ice_single_port != 0 {
            let rv = UdpSocket::bind(("0.0.0.0", c.webrtc.ice_single_port as u16)).await;
            let udp_socket: UdpSocket;
            match rv {
                Ok(sock) => {
                    udp_socket = sock;
                }
                Err(_) => {
                    std::process::exit(0);
                }
            }
            let udp_mux = UDPMuxDefault::new(UDPMuxParams::new(udp_socket));
            se.set_udp_network(UDPNetwork::Muxed(udp_mux));
        } else {
            let mut ice_port_start: u16 = 0;
            let mut ice_port_end: u16 = 0;

            if c.turn.enabled && c.turn.port_range.len() == 0 {
                ice_port_start = super::turn::SFU_MIN_PORT;
                ice_port_end = super::turn::SFU_MAX_PORT;
            } else if c.webrtc.ice_port_range.len() == 2 {
                ice_port_start = c.webrtc.ice_port_range[0];
                ice_port_end = c.webrtc.ice_port_range[1];
            }

            if ice_port_start != 0 || ice_port_end != 0 {
                let ephemeral_udp =
                    UDPNetwork::Ephemeral(EphemeralUDP::new(ice_port_start, ice_port_end).unwrap());
                se.set_udp_network(ephemeral_udp);
            }
        }

        let mut ice_servers: Vec<RTCIceServer> = Vec::new();
        if c.webrtc.candidates.ice_lite {
            se.set_lite(c.webrtc.candidates.ice_lite);
        } else {
            for ice_server in &c.webrtc.ice_servers {
                let s = RTCIceServer {
                    urls: ice_server.urls.clone(),
                    username: ice_server.user_name.clone(),
                    credential: ice_server.credential.clone(),
                    credential_type: RTCIceCredentialType::Unspecified,
                };

                ice_servers.push(s);
            }
        }
        let mut sdp_semantics = RTCSdpSemantics::UnifiedPlan;

        match c.webrtc.sdp_semantics.as_str() {
            "unified-plan-with-fallback" => {
                sdp_semantics = RTCSdpSemantics::UnifiedPlanWithFallback;
            }
            "plan-b" => {
                sdp_semantics = RTCSdpSemantics::PlanB;
            }
        }

        if c.webrtc.timeouts.ice_disconnected_timeout == 0
            && c.webrtc.timeouts.ice_failed_timeout == 0
            && c.webrtc.timeouts.ice_keepalive_interval == 0
        {
        } else {
            se.set_ice_timeouts(
                Some(Duration::from_secs(
                    c.webrtc.timeouts.ice_disconnected_timeout as u64,
                )),
                Some(Duration::from_secs(
                    c.webrtc.timeouts.ice_failed_timeout as u64,
                )),
                Some(Duration::from_secs(
                    c.webrtc.timeouts.ice_keepalive_interval as u64,
                )),
            );
        }

        let mut w = WebRTCTransportConfig {
            configuration: RTCConfiguration {
                ice_servers: ice_servers,
                sdp_semantics: sdp_semantics,
                ..Default::default()
            },
            setting: se,
            Router: c.router.clone(),
        };

        if c.webrtc.candidates.nat1_to_1ips.len() > 0 {
            w.setting.set_nat_1to1_ips(
                c.webrtc.candidates.nat1_to_1ips.clone(),
                RTCIceCandidateType::Host,
            );
        }

        if c.webrtc.mdns {
            w.setting
                .set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);
        }

        if c.sfu.with_stats {
            w.Router.with_stats = true;
        }

        Ok(w)
    }
}

impl SFU {
    async fn new(c: Config) -> Result<Self> {
        let w = WebRTCTransportConfig::new(&c).await.unwrap();

        let with_status = w.Router.with_stats;

        let mut sfu = SFU {
            webrtc: w,
            sessions: HashMap::new(),
            with_status: with_status,
            ..Default::default()
        };

        if c.turn.enabled {
            let turn_server = super::turn::init_turn_server(c.turn, c.turn_auth).await?;
            sfu.turn = Some(turn_server);
        }

        Ok(sfu)
    }
}
