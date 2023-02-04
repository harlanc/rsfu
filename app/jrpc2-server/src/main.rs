use anyhow::Result;
use jrpc2::jsonrpc2::JsonRpc2;
use jrpc2::stream_ws::ServerObjectStream;
use jrpc2_server::server::signal::JsonSignal;
use rsfu::sfu::peer::PeerLocal;
use rsfu::sfu::sfu;
use rsfu::sfu::sfu::SFU;
use std::env;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<()> {
    env::set_var("RUST_LOG", "info");
    env_logger::init();

    let config_path = String::from("./app/jrpc2-server/src/config.toml");
    let config = sfu::load(&config_path);
    match config {
        Err(err) => {
            log::error!("configure error: {}", err);
            log::warn!("please check the configure file path: {}", config_path);
        }
        Ok(c) => {
            let sfu = SFU::new(c).await.unwrap();
            sfu.new_data_channel(String::from("rsfu")).await;

            let arc_sfu = Arc::new(Mutex::new(sfu));
            let addr = "127.0.0.1:7000";
            let listener = TcpListener::bind(&addr).await.expect("Can't listen");

            log::info!("listen: {}", addr);

            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    let server_stream = ServerObjectStream::accept(stream)
                        .await
                        .expect("cannot generate object stream");
                    let peer_local = PeerLocal::new(arc_sfu.clone());
                    JsonRpc2::new(
                        Box::new(server_stream),
                        Some(Box::new(JsonSignal::new(peer_local))),
                    )
                    .await;
                }
            }
        }
    }

    signal::ctrl_c().await?;

    Ok(())
}
