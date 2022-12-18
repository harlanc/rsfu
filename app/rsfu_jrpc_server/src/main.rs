use anyhow::Result;
use jrpc2::jsonrpc2::JsonRpc2;
use jrpc2::stream_ws::ServerObjectStream;
use rsfu::sfu::peer::PeerLocal;
use rsfu::sfu::sfu::Config;
use rsfu::sfu::sfu::SFU;
use std::sync::Arc;
use tokio;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
// use rsfu-jrpc-server::server::JsonSignal;
use rsfu_jrpc_server::server::server::JsonSignal;
use std::env;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::default();

    env::set_var("RUST_LOG", "info");
    env_logger::init();

    let sfu = SFU::new(config).await.unwrap();
    sfu.new_data_channel(String::from("rsfu")).await;

    let addr = "127.0.0.1:7000";
    let listener = TcpListener::bind(&addr).await.expect("Can't listen");

    let peer_local = PeerLocal::new(Arc::new(Mutex::new(sfu)));
    log::info!("testsetst");

    if let Ok((stream, _)) = listener.accept().await {
        let server_stream = ServerObjectStream::accept(stream)
            .await
            .expect("cannot generate object stream");

        JsonRpc2::new(
            Box::new(server_stream),
            Some(Box::new(JsonSignal { peer_local })),
        )
        .await;
    }

    signal::ctrl_c().await?;

    Ok(())
}
