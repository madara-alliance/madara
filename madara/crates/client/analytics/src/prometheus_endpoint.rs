use anyhow::Context;
use http_body_util::Full;
use hyper::{body::Bytes, server::conn::http1, service::service_fn, Response};
use hyper_util::rt::TokioIo;
use mp_utils::service::ServiceContext;
use opentelemetry_prometheus::PrometheusExporter;
use prometheus::Encoder;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct PrometheusEndpointConfig {
    pub enabled: bool,
    pub listen_external: bool,
    pub listen_port: u16,
}

impl Default for PrometheusEndpointConfig {
    fn default() -> Self {
        Self { enabled: false, listen_external: false, listen_port: 9464 }
    }
}

pub(crate) struct PrometheusEndpoint {
    pub exporter: Option<PrometheusExporter>,
    config: PrometheusEndpointConfig,
    registry: prometheus::Registry,
}

impl PrometheusEndpoint {
    pub fn new(config: PrometheusEndpointConfig) -> anyhow::Result<Self> {
        let registry = prometheus::Registry::new();

        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()
            .context("Building opentelemetry_prometheus")?;
        Ok(Self { exporter: Some(exporter), config, registry })
    }

    pub async fn run(&mut self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let listen_addr = if self.config.listen_external {
            Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
        } else {
            Ipv4Addr::LOCALHOST
        };
        let addr = SocketAddr::new(listen_addr.into(), self.config.listen_port);
        let listener = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;

        tracing::info!("🌐 Running prometheus telemetry endpoint at http://{}/metrics", addr);

        while let Some(res) = ctx.run_until_cancelled(listener.accept()).await {
            let (stream, _addr) = match res {
                Ok(res) => res,
                Err(err) => {
                    tracing::error!("Error accepting http connection: {:#}", err);
                    continue;
                }
            };
            let io = TokioIo::new(stream);

            let registry = self.registry.clone();
            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let registry = registry.clone();
                    async move {
                        if req.uri().path() == "/metrics" {
                            // L'exportateur a une méthode pour accéder au registre
                            let metrics = registry.gather();

                            let mut buffer = vec![];
                            let encoder = prometheus::TextEncoder::new();
                            encoder.encode(&metrics, &mut buffer).context("Encoding prometheus metrics")?;

                            Response::builder()
                                .status(200)
                                .header("Content-Type", "text/plain; charset=utf-8")
                                .body(Full::new(Bytes::from(buffer)))
                                .context("Making http response")
                        } else {
                            Response::builder()
                                .status(404)
                                .body(Full::new(Bytes::from("Not Found - See /metrics for metrics.")))
                                .context("Making http 404 response")
                        }
                    }
                });

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("Error serving connection: {:#}", err);
                }
            });
        }

        Ok(())
    }
}
