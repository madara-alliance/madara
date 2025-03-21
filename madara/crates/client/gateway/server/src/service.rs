use super::router::main_router;
use anyhow::Context;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;
use mp_utils::service::ServiceContext;
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct GatewayServerConfig {
    pub feeder_gateway_enable: bool,
    pub gateway_enable: bool,
    pub gateway_external: bool,
    pub gateway_port: u16,
    pub enable_trusted_add_verified_transaction: bool,
}
impl Default for GatewayServerConfig {
    fn default() -> Self {
        Self {
            feeder_gateway_enable: false,
            gateway_enable: false,
            gateway_external: false,
            gateway_port: 8080,
            enable_trusted_add_verified_transaction: false,
        }
    }
}

pub async fn start_server(
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    mut ctx: ServiceContext,
    config: GatewayServerConfig,
) -> anyhow::Result<()> {
    if !config.feeder_gateway_enable && !config.gateway_enable && !config.enable_trusted_add_verified_transaction {
        return Ok(());
    }

    let listen_addr = if config.gateway_external {
        Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
    } else {
        Ipv4Addr::LOCALHOST
    };
    let addr = SocketAddr::new(listen_addr.into(), config.gateway_port);
    let listener = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;

    tracing::info!("🌐 Gateway endpoint started at {}", addr);

    while let Some(res) = ctx.run_until_cancelled(listener.accept()).await {
        // Handle new incoming connections
        if let Ok((stream, _)) = res {
            let io = TokioIo::new(stream);

            let db_backend = Arc::clone(&db_backend);
            let add_transaction_provider = add_transaction_provider.clone();
            let ctx = ctx.clone();
            let config = config.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    main_router(
                        req,
                        Arc::clone(&db_backend),
                        add_transaction_provider.clone(),
                        ctx.clone(),
                        config.clone(),
                    )
                });

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("Error serving connection: {:#}", err);
                }
            });
        }
    }

    Ok(())
}
