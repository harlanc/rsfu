use super::router::RouterConfig;

use super::turn::TurnConfig;

use webrtc::api::setting_engine::SettingEngine;
use webrtc::peer_connection::configuration::RTCConfiguration;

use super::data_channel::DataChannel;
use super::session::{Session, SessionLocal};
use anyhow::Result;

use std::collections::HashMap;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use turn::server::Server as TurnServer;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;

use super::peer::SessionProvider;
use crate::buffer::factory::AtomicFactory;
use serde::Deserialize;
use std::sync::Arc;
use turn::auth::AuthHandler;

use webrtc::peer_connection::policy::sdp_semantics::RTCSdpSemantics;
use webrtc_ice::mdns::MulticastDnsMode;
use webrtc_ice::udp_mux::*;
use webrtc_ice::udp_network::*;

use super::errors::ConfigError;
use async_trait::async_trait;
use std::fs;
#[derive(Clone, Deserialize)]
struct ICEServerConfig {
    urls: Vec<String>,
    user_name: String,
    credential: String,
}
#[derive(Clone, Default, Deserialize)]
struct Candidates {
    #[serde(rename = "icelite")]
    ice_lite: Option<bool>,
    #[serde(rename = "nat1to1ips")]
    nat1_to_1ips: Option<Vec<String>>,
}
#[derive(Default)]
pub struct WebRTCTransportConfig {
    pub configuration: RTCConfiguration,
    pub setting: SettingEngine,
    pub router: RouterConfig,
    pub factory: Arc<Mutex<AtomicFactory>>,
}
#[derive(Clone, Default, Deserialize)]
struct WebRTCTimeoutsConfig {
    #[serde(rename = "disconnected")]
    ice_disconnected_timeout: i32,
    #[serde(rename = "failed")]
    ice_failed_timeout: i32,
    #[serde(rename = "keepalive")]
    ice_keepalive_interval: i32,
}
#[derive(Clone, Default, Deserialize)]
pub struct WebRTCConfig {
    ice_single_port: Option<i32>,
    #[serde(rename = "portrange")]
    pub ice_port_range: Option<Vec<u16>>,
    ice_servers: Option<Vec<ICEServerConfig>>,
    candidates: Candidates,
    #[serde(rename = "sdpsemantics")]
    pub sdp_semantics: String,
    #[serde(rename = "mdns")]
    mdns: bool,
    timeouts: WebRTCTimeoutsConfig,
}
#[derive(Clone, Default, Deserialize)]
struct SFUConfig {
    #[allow(dead_code)]
    #[serde(rename = "ballast")]
    ballast: i64,
    #[serde(rename = "withstats")]
    with_stats: bool,
}
#[derive(Clone, Default, Deserialize)]
pub struct Config {
    sfu: SFUConfig,
    router: RouterConfig,
    pub webrtc: WebRTCConfig,
    turn: TurnConfig,
    #[serde(skip_deserializing)]
    turn_auth: Option<Arc<dyn AuthHandler + Send + Sync>>,
}

pub fn load(cfg_path: &String) -> Result<Config, ConfigError> {
    let content = fs::read_to_string(cfg_path)?;
    let decoded_config = toml::from_str(&content[..]).unwrap();
    Ok(decoded_config)
}

#[derive(Default)]
pub struct SFU {
    webrtc: Arc<WebRTCTransportConfig>,
    turn: Option<TurnServer>,
    sessions: Arc<Mutex<HashMap<String, Arc<dyn Session + Send + Sync>>>>,
    data_channels: Arc<Mutex<Vec<Arc<DataChannel>>>>,
    #[allow(dead_code)]
    with_status: bool,
}

impl WebRTCTransportConfig {
    async fn new(c: &Config) -> Result<Self> {
        let mut se = SettingEngine::default();
        se.disable_media_engine_copy(true);

        if let Some(ice_single_port) = c.webrtc.ice_single_port {
            let rv = UdpSocket::bind(("0.0.0.0", ice_single_port as u16)).await;
            let udp_socket: UdpSocket = match rv {
                Ok(sock) => sock,
                Err(_) => {
                    std::process::exit(0);
                }
            };
            let udp_mux = UDPMuxDefault::new(UDPMuxParams::new(udp_socket));
            se.set_udp_network(UDPNetwork::Muxed(udp_mux));
        } else {
            let mut ice_port_start: u16 = 0;
            let mut ice_port_end: u16 = 0;

            if c.turn.enabled && c.turn.port_range.is_none() {
                ice_port_start = super::turn::SFU_MIN_PORT;
                ice_port_end = super::turn::SFU_MAX_PORT;
            } else if let Some(ice_port_range) = &c.webrtc.ice_port_range {
                if ice_port_range.len() == 2 {
                    ice_port_start = ice_port_range[0];
                    ice_port_end = ice_port_range[1];
                }
            }

            if ice_port_start != 0 || ice_port_end != 0 {
                let ephemeral_udp =
                    UDPNetwork::Ephemeral(EphemeralUDP::new(ice_port_start, ice_port_end).unwrap());
                se.set_udp_network(ephemeral_udp);
            }
        }

        let mut ice_servers: Vec<RTCIceServer> = Vec::new();
        if let Some(ice_lite) = c.webrtc.candidates.ice_lite {
            if ice_lite {
                se.set_lite(ice_lite);
            } else if let Some(ice_servers_cfg) = &c.webrtc.ice_servers {
                for ice_server in ice_servers_cfg {
                    let s = RTCIceServer {
                        urls: ice_server.urls.clone(),
                        username: ice_server.user_name.clone(),
                        credential: ice_server.credential.clone(),
                        credential_type: RTCIceCredentialType::Unspecified,
                    };

                    ice_servers.push(s);
                }
            }
        }

        let mut _sdp_semantics = RTCSdpSemantics::UnifiedPlan;

        match c.webrtc.sdp_semantics.as_str() {
            "unified-plan-with-fallback" => {
                _sdp_semantics = RTCSdpSemantics::UnifiedPlanWithFallback;
            }
            "plan-b" => {
                _sdp_semantics = RTCSdpSemantics::PlanB;
            }
            _ => {}
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
                ice_servers,
                ..Default::default()
            },
            setting: se,
            router: c.router.clone(),
            factory: Arc::new(Mutex::new(AtomicFactory::new(1000, 1000))),
        };

        if let Some(nat1toiips) = &c.webrtc.candidates.nat1_to_1ips {
            if !nat1toiips.is_empty() {
                w.setting
                    .set_nat_1to1_ips(nat1toiips.clone(), RTCIceCandidateType::Host);
            }
        }

        if c.webrtc.mdns {
            w.setting
                .set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);
        }

        if c.sfu.with_stats {
            w.router.with_stats = true;
        }

        Ok(w)
    }
}

impl SFU {
    pub async fn new(c: Config) -> Result<Self> {
        let w = Arc::new(WebRTCTransportConfig::new(&c).await.unwrap());

        let with_status = w.router.with_stats;

        let mut sfu = SFU {
            webrtc: w,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            with_status,
            ..Default::default()
        };

        if c.turn.enabled {
            let turn_server = super::turn::init_turn_server(c.turn, c.turn_auth).await?;
            sfu.turn = Some(turn_server);
        }

        Ok(sfu)
    }

    async fn new_session(&self, id: String) -> Arc<dyn Session + Send + Sync> {
        let session =
            SessionLocal::new(id.clone(), self.data_channels.clone(), self.webrtc.clone()).await;

        let sessions_out = self.sessions.clone();
        let id_out = id.clone();
        session
            .on_close(Box::new(move || {
                let sessions_in = sessions_out.clone();
                let id_in = id_out.clone();
                Box::pin(async move {
                    sessions_in.lock().await.remove(&id_in);
                })
            }))
            .await;

        self.sessions.lock().await.insert(id, session.clone());

        session
    }

    pub async fn new_data_channel(&self, label: String) -> Arc<DataChannel> {
        let dc = Arc::new(DataChannel::new(label));
        self.data_channels.lock().await.push(dc.clone());
        dc
    }
}

#[async_trait]
impl SessionProvider for SFU {
    async fn get_session(
        &self,
        sid: String,
    ) -> (
        Option<Arc<dyn Session + Send + Sync>>,
        Arc<WebRTCTransportConfig>,
    ) {
        if let Some(session) = self.sessions.lock().await.get(&sid) {
            return (Some(session.clone()), self.webrtc.clone());
        }

        let session = self.new_session(sid).await;
        return (Some(session), self.webrtc.clone());
    }
}
