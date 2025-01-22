use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::Context;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::service::ServiceContext;
use tokio::net::TcpListener;

use super::router::main_router;

pub async fn start_server(
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
    gateway_external: bool,
    gateway_port: u16,
    mut ctx: ServiceContext,
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

    while let Some(res) = ctx.run_until_cancelled(listener.accept()).await {
        // Handle new incoming connections
        if let Ok((stream, _)) = res {
            let io = TokioIo::new(stream);

            let db_backend = Arc::clone(&db_backend);
            let add_transaction_provider = add_transaction_provider.clone();
            let ctx = ctx.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    main_router(
                        req,
                        Arc::clone(&db_backend),
                        add_transaction_provider.clone(),
                        ctx.clone(),
                        feeder_gateway_enable,
                        gateway_enable,
                    )
                });

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("Error serving connection: {:?}", err);
                }
            });
        }
    }

    anyhow::Ok(())
}
