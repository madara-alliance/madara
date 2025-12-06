use super::router::main_router;
use anyhow::Context;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use mc_db::MadaraBackend;
use mc_submit_tx::{SubmitTransaction, SubmitValidatedTransaction};
use mp_utils::service::ServiceContext;
use std::{
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Instant,
};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct GatewayServerConfig {
    pub feeder_gateway_enable: bool,
    pub gateway_enable: bool,
    pub gateway_external: bool,
    pub gateway_port: u16,
    pub enable_trusted_add_validated_transaction: bool,
}
impl Default for GatewayServerConfig {
    fn default() -> Self {
        Self {
            feeder_gateway_enable: false,
            gateway_enable: false,
            gateway_external: false,
            gateway_port: 8080,
            enable_trusted_add_validated_transaction: false,
        }
    }
}

pub async fn start_server(
    db_backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
    submit_validated: Option<Arc<dyn SubmitValidatedTransaction>>,
    mut ctx: ServiceContext,
    config: GatewayServerConfig,
) -> anyhow::Result<()> {
    if !config.feeder_gateway_enable && !config.gateway_enable && !config.enable_trusted_add_validated_transaction {
        return Ok(());
    }

    let listen_addr = if config.gateway_external {
        Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
    } else {
        Ipv4Addr::LOCALHOST
    };
    let addr = SocketAddr::new(listen_addr.into(), config.gateway_port);
    let listener = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;

    let addr = listener.local_addr().context("Getting the bound-to address.")?;
    tracing::info!("🌐 Gateway endpoint started at {}", addr);

    while let Some(res) = ctx.run_until_cancelled(listener.accept()).await {
        // Handle new incoming connections
        if let Ok((stream, _)) = res {
            let io = TokioIo::new(stream);

            let db_backend = Arc::clone(&db_backend);
            let add_transaction_provider = add_transaction_provider.clone();
            let submit_validated = submit_validated.clone();
            let config = config.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let db_backend = Arc::clone(&db_backend);
                    let add_transaction_provider = add_transaction_provider.clone();
                    let submit_validated = submit_validated.clone();
                    let config = config.clone();
                    async move {
                        let path = req
                            .uri()
                            .path()
                            .split('/')
                            .filter(|segment| !segment.is_empty())
                            .collect::<Vec<_>>()
                            .join("/");
                        let start = Instant::now();
                        let Ok(res) =
                            main_router(req, &path, db_backend, add_transaction_provider, submit_validated, config)
                                .await;

                        let status = res.status().as_u16() as i64;
                        let res_len = res.body().len() as u64;
                        let response_time = start.elapsed().as_micros();

                        tracing::debug!(
                            target: "gateway_calls",
                            method = &path,
                            status = status,
                            res_len = res_len,
                            response_time = response_time,
                            "{path} {status} {res_len} - {response_time} micros"
                        );

                        Ok::<_, Infallible>(res)
                    }
                });

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("Error serving connection: {:#}", err);
                }
            });
        }
    }

    Ok(())
}
