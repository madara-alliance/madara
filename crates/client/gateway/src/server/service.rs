use std::{
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::Context;
use hyper::{
    service::{make_service_fn, service_fn},
    Server,
};
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::graceful_shutdown;
use tokio::net::TcpListener;

use super::router::main_router;

pub async fn start_server(
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
) -> anyhow::Result<()> {
    if !feeder_gateway_enable && !gateway_enable {
        return Ok(());
    }

    let listen_addr = if gateway_external {
        Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
    } else {
        Ipv4Addr::LOCALHOST
    };
    let addr = SocketAddr::new(listen_addr.into(), gateway_port);

    let socket = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;

    let listener = hyper::server::conn::AddrIncoming::from_listener(socket)
        .with_context(|| format!("Opening socket server at {addr}"))?;

    let make_service = make_service_fn(move |_| {
        let db_backend = Arc::clone(&db_backend);
        let add_transaction_provider = Arc::clone(&add_transaction_provider);
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                main_router(
                    req,
                    Arc::clone(&db_backend),
                    Arc::clone(&add_transaction_provider),
                    feeder_gateway_enable,
                    gateway_enable,
                )
            }))
        }
    });

    log::info!("🌐 Gateway endpoint started at {}", listener.local_addr());

    let server = Server::builder(listener).serve(make_service).with_graceful_shutdown(graceful_shutdown());

    server.await.context("gateway server")?;

    Ok(())
}
