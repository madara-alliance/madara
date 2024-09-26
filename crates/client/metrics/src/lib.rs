use std::net::{Ipv4Addr, SocketAddr};

use anyhow::Context;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server, StatusCode,
};
use mp_utils::{service::Service, wait_or_graceful_shutdown, StopHandle};
use prometheus::{core::Collector, Encoder, TextEncoder};
use tokio::{net::TcpListener, sync::oneshot, task::JoinSet};

pub use prometheus::{
    self,
    core::{
        AtomicF64 as F64, AtomicI64 as I64, AtomicU64 as U64, GenericCounter as Counter,
        GenericCounterVec as CounterVec, GenericGauge as Gauge, GenericGaugeVec as GaugeVec,
    },
    exponential_buckets, Error as PrometheusError, Histogram, HistogramOpts, HistogramVec, IntGaugeVec, Opts, Registry,
};

#[derive(Clone, Debug)]
/// This sturct can be cloned cheaply, and will still point to the same registry.
pub struct MetricsRegistry(Option<Registry>); // Registry is already an Arc

impl MetricsRegistry {
    pub fn register<T: Clone + Collector + 'static>(&self, metric: T) -> Result<T, PrometheusError> {
        if let Some(reg) = &self.0 {
            reg.register(Box::new(metric.clone()))?;
            Ok(metric)
        } else {
            Ok(metric)
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    /// Make a dummy registry that does nothing. Useful for wiring up metrics in tests.
    pub fn dummy() -> Self {
        Self(None)
    }
}

#[derive(thiserror::Error, Debug)]
#[error("error while handling request in prometheus endpoint: {0}")]
enum Error {
    Prometheus(#[from] prometheus::Error),
    Hyper(#[from] hyper::Error),
    HyperHttp(#[from] hyper::http::Error),
}

async fn endpoint(req: Request<Body>, registry: Registry) -> Result<Response<Body>, Error> {
    if req.uri().path() == "/metrics" {
        let metric_families = registry.gather();
        let mut buffer = vec![];
        let encoder = TextEncoder::new();
        encoder.encode(&metric_families, &mut buffer)?;

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", encoder.format_type())
            .body(Body::from(buffer))?)
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/html")
            .body(Body::from("Not found.<br><br><a href=\"/metrics\">See Metrics</a>"))?)
    }
}

pub struct MetricsService {
    no_prometheus: bool,
    prometheus_external: bool,
    prometheus_port: u16,
    registry: MetricsRegistry,
    stop_handle: StopHandle,
}

impl MetricsService {
    pub fn new(no_prometheus: bool, prometheus_external: bool, prometheus_port: u16) -> anyhow::Result<Self> {
        Ok(Self {
            no_prometheus,
            prometheus_external,
            prometheus_port,
            registry: MetricsRegistry(if no_prometheus { None } else { Some(Default::default()) }),
            stop_handle: Default::default(),
        })
    }

    pub fn registry(&self) -> &MetricsRegistry {
        &self.registry
    }
}

#[async_trait::async_trait]
impl Service for MetricsService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if self.no_prometheus {
            return Ok(());
        }

        let listen_addr = if self.prometheus_external {
            Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
        } else {
            Ipv4Addr::LOCALHOST
        };
        let addr = SocketAddr::new(listen_addr.into(), self.prometheus_port);

        let registry = self.registry.clone();
        let service = make_service_fn(move |_| {
            let registry = registry.clone();
            async move {
                Ok::<_, hyper::Error>(service_fn(move |req: Request<Body>| {
                    let registry = registry.clone();
                    async move {
                        match endpoint(req, registry.0.expect("Registry should not be none").clone()).await {
                            Ok(res) => Ok::<_, Error>(res),
                            Err(err) => {
                                log::error!("Error when handling prometheus request: {}", err);
                                Ok(Response::builder()
                                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                                    .body(Body::from("Internal server error"))?)
                            }
                        }
                    }
                }))
            }
        });

        let (stop_send, stop_recv) = oneshot::channel();
        self.stop_handle = StopHandle::new(Some(stop_send));

        join_set.spawn(async move {
            let socket = TcpListener::bind(addr).await.with_context(|| format!("Opening socket server at {addr}"))?;
            let listener = hyper::server::conn::AddrIncoming::from_listener(socket)
                .with_context(|| format!("Opening socket server at {addr}"))?;
            log::info!("📈 Prometheus endpoint started at {}", listener.local_addr());
            let server = Server::builder(listener).serve(service).with_graceful_shutdown(async {
                wait_or_graceful_shutdown(stop_recv).await;
            });
            server.await.context("Running prometheus server")?;
            Ok(())
        });

        Ok(())
    }
}
