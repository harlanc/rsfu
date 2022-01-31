use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::time::Duration;
use turn::relay::relay_range::RelayAddressGeneratorRanges;
use turn::server::config::ConnConfig;
use turn::server::config::ServerConfig;
use turn::server::Server as TurnServer;

use std::sync::Arc;
use turn::auth;
use turn::auth::AuthHandler;
use turn::auth::LongTermAuthHandler;
use turn::Error;

use anyhow::Result as AnyhowResult;
use regex::Regex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::result::Result;

use std::str::FromStr;
use webrtc_util::vnet::net::*;

//use util::vnet::net::

pub const TURN_MIN_PORT: u16 = 32768;
pub const TURN_MAX_PORT: u16 = 46883;

pub const SFU_MIN_PORT: u16 = 46884;
pub const SFU_MAX_PORT: u16 = 60999;
#[derive(Clone)]
pub(super) struct TurnAuth {
    credentials: String,
    secret: String,
}
#[derive(Clone)]
pub(super) struct TurnConfig {
    pub(super) enabled: bool,
    realm: String,
    address: String,
    cert: String,
    key: String,
    auth: TurnAuth,
    pub(super) port_range: Vec<u16>,
}

struct CustomAuthHandler {
    users_map: HashMap<String, Vec<u8>>,
}
impl CustomAuthHandler {
    fn new(user_map: HashMap<String, Vec<u8>>) -> Self {
        Self {
            users_map: user_map,
        }
    }
}
impl AuthHandler for CustomAuthHandler {
    fn auth_handle(
        &self,
        username: &str,
        realm: &str,
        _src_addr: SocketAddr,
    ) -> Result<Vec<u8>, Error> {
        if let Some(val) = self.users_map.get(&username.to_string()) {
            return Ok(val.clone());
        }

        Err(Error::ErrNilConn)
    }
}

pub(super) async fn init_turn_server(
    conf: TurnConfig,
    auth: Option<Arc<dyn AuthHandler + Send + Sync>>,
) -> AnyhowResult<TurnServer> {
    let conn = Arc::new(UdpSocket::bind(conf.address.clone()).await?);
    println!("listening {}...", conn.local_addr()?);

    let mut new_auth: Option<Arc<dyn AuthHandler + Send + Sync>> = auth;

    if new_auth.is_none() {
        if conf.auth.secret != "" {
            new_auth = Some(Arc::new(LongTermAuthHandler::new(conf.auth.secret)));
        } else {
            let mut users_map: HashMap<String, Vec<u8>> = HashMap::new();
            let re = Regex::new(r"(\w+)=(\w+)").unwrap();

            for caps in re.captures_iter(conf.realm.clone().as_str()) {
                let username = caps.get(1).unwrap().as_str();
                let username_String = username.to_string();
                let password = caps.get(2).unwrap().as_str();
                users_map.insert(
                    username_String,
                    auth::generate_auth_key(username, conf.realm.clone().as_str(), password),
                );
            }

            if users_map.len() == 0 {
                log::error!("no turn auth provided.");
            }

            new_auth = Some(Arc::new(CustomAuthHandler::new(users_map)))
        }
    }

    let mut min_port: u16 = TURN_MIN_PORT;
    let mut max_port: u16 = TURN_MAX_PORT;

    if conf.port_range.len() == 2 {
        min_port = conf.port_range[0];
        max_port = conf.port_range[1];
    }

    let addr: Vec<&str> = conf.address.split(':').collect();

    let mut conn_configs = Vec::new();
    conn_configs.push(ConnConfig {
        conn,
        relay_addr_generator: Box::new(RelayAddressGeneratorRanges {
            min_port,
            max_port,
            max_retries: 1,
            relay_address: IpAddr::from_str(addr[0])?,
            address: "0.0.0.0".to_owned(),
            net: Arc::new(Net::new(Some(NetConfig::default()))),
        }),
    });

    let turn_server = TurnServer::new(ServerConfig {
        conn_configs,
        realm: conf.realm,
        auth_handler: new_auth.unwrap(),
        channel_bind_timeout: Duration::from_secs(5),
    })
    .await?;
    Ok(turn_server)
}
