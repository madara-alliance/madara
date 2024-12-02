use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::Context;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::{graceful_shutdown, service::ServiceContext};
use tokio::{net::TcpListener, sync::Notify};

use super::router::main_router;

pub async fn start_server(
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
    ctx: ServiceContext,
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
    let listener = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;

    tracing::info!("🌐 Gateway endpoint started at {}", addr);

    let shutdown_notify = Arc::new(Notify::new());

    {
        let shutdown_notify = Arc::clone(&shutdown_notify);
        tokio::spawn(async move {
            graceful_shutdown(&ctx).await;
            shutdown_notify.notify_waiters();
        });
    }

    loop {
        tokio::select! {
            // Handle new incoming connections
            Ok((stream, _)) = listener.accept() => {
                let io = TokioIo::new(stream);

                let db_backend = Arc::clone(&db_backend);
                let add_transaction_provider = Arc::clone(&add_transaction_provider);

                tokio::task::spawn(async move {
                    let service = service_fn(move |req| {
                        main_router(
                            req,
                            Arc::clone(&db_backend),
                            Arc::clone(&add_transaction_provider),
                            feeder_gateway_enable,
                            gateway_enable,
                        )
                    });

                    if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                        tracing::error!("Error serving connection: {:?}", err);
                    }
                });
            },

            // Await the shutdown signal
            _ = shutdown_notify.notified() => {
                break Ok(());
            }
        }
    }
}
