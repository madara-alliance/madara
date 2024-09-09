//! JSON-RPC specific middleware.

use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::future::{BoxFuture, FutureExt};
use governor::clock::{Clock, DefaultClock, QuantaClock};
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter};
use hyper::{Body, Response};
use jsonrpsee::server::middleware::rpc::RpcServiceT;
use jsonrpsee::types::{ErrorObject, Request};
use jsonrpsee::MethodResponse;
use serde_json::{json, Value};
use tower::{Layer, Service};

use mp_chain_config::{RpcVersion, RpcVersionError};

pub use super::metrics::{Metrics, RpcMetrics};

/// Rate limit middleware
#[derive(Debug, Clone)]
pub struct RateLimit {
    pub(crate) inner: Arc<governor::RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
    pub(crate) clock: QuantaClock,
}

impl RateLimit {
    pub fn new(max_burst: NonZeroU32) -> Self {
        let clock = QuantaClock::default();
        Self { inner: Arc::new(RateLimiter::direct_with_clock(Quota::per_minute(max_burst), &clock)), clock }
    }
}

const MAX_JITTER: Duration = Duration::from_millis(50);
const MAX_RETRIES: usize = 10;

#[derive(Debug, Clone, Default)]
pub struct MiddlewareLayer {
    rate_limit: Option<RateLimit>,
    metrics: Option<Metrics>,
}

impl MiddlewareLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable new rate limit middleware enforced per minute.
    pub fn with_rate_limit_per_minute(self, n: NonZeroU32) -> Self {
        Self { rate_limit: Some(RateLimit::new(n)), metrics: self.metrics }
    }

    /// Enable metrics middleware.
    pub fn with_metrics(self, metrics: Metrics) -> Self {
        Self { rate_limit: self.rate_limit, metrics: Some(metrics) }
    }

    /// Register a new websocket connection.
    pub fn ws_connect(&self) {
        if let Some(m) = self.metrics.as_ref() {
            m.ws_connect()
        }
    }

    /// Register that a websocket connection was closed.
    pub fn ws_disconnect(&self, now: Instant) {
        if let Some(m) = self.metrics.as_ref() {
            m.ws_disconnect(now)
        }
    }
}

impl<S> tower::Layer<S> for MiddlewareLayer {
    type Service = Middleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        Middleware { service, rate_limit: self.rate_limit.clone(), metrics: self.metrics.clone() }
    }
}

pub struct Middleware<S> {
    service: S,
    rate_limit: Option<RateLimit>,
    metrics: Option<Metrics>,
}

impl<'a, S> RpcServiceT<'a> for Middleware<S>
where
    S: Send + Sync + RpcServiceT<'a> + Clone + 'static,
{
    type Future = BoxFuture<'a, MethodResponse>;

    fn call(&self, req: Request<'a>) -> Self::Future {
        let now = Instant::now();

        if let Some(m) = self.metrics.as_ref() {
            m.on_call(&req)
        }

        let service = self.service.clone();
        let rate_limit = self.rate_limit.clone();
        let metrics = self.metrics.clone();

        async move {
            let mut is_rate_limited = false;

            if let Some(limit) = rate_limit.as_ref() {
                let mut attempts = 0;
                let jitter = Jitter::up_to(MAX_JITTER);

                loop {
                    if attempts >= MAX_RETRIES {
                        return MethodResponse::error(
                            req.id,
                            ErrorObject::owned(-32999, "RPC rate limit exceeded", None::<()>),
                        );
                    }

                    if let Err(rejected) = limit.inner.check() {
                        tokio::time::sleep(jitter + rejected.wait_time_from(limit.clock.now())).await;
                    } else {
                        break;
                    }

                    is_rate_limited = true;
                    attempts += 1;
                }
            }

            let rp = service.call(req.clone()).await;

            let method = req.method_name();
            let status = rp.as_error_code().unwrap_or(200);
            let res_len = rp.as_result().len();
            let response_time = now.elapsed();

            log::info!(
                target: "rpc_calls",
                method = method,
                status = status,
                res_len = res_len,
                response_time = response_time.as_micros();
                "{method} {status} {res_len} - {response_time:?}",
            );

            if let Some(m) = metrics.as_ref() {
                m.on_response(&req, &rp, is_rate_limited, now)
            }

            rp
        }
        .boxed()
    }
}

#[derive(Clone)]
pub struct VersionMiddleware<S> {
    inner: S,
}

#[derive(thiserror::Error, Debug)]
enum VersionMiddlewareError {
    #[error("Failed to read request body: {0}")]
    BodyReadError(#[from] hyper::Error),
    #[error("Failed to parse JSON: {0}")]
    JsonParseError(#[from] serde_json::Error),
    #[error("Invalid request format")]
    InvalidRequestFormat,
    #[error("Invalid URL format")]
    InvalidUrlFormat,
    #[error("Invalid version specified")]
    InvalidVersion,
    #[error("Unsupported version specified")]
    UnsupportedVersion,
}

impl From<RpcVersionError> for VersionMiddlewareError {
    fn from(e: RpcVersionError) -> Self {
        match e {
            RpcVersionError::InvalidNumber(_) => Self::InvalidVersion,
            RpcVersionError::InvalidPathSupplied => Self::InvalidUrlFormat,
            RpcVersionError::InvalidVersion => Self::InvalidVersion,
            RpcVersionError::TooManyComponents(_) => Self::InvalidVersion,
            RpcVersionError::UnsupportedVersion => Self::UnsupportedVersion,
        }
    }
}

impl<S> VersionMiddleware<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

#[derive(Clone)]
pub struct VersionMiddlewareLayer;

impl<S> Layer<S> for VersionMiddlewareLayer {
    type Service = VersionMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        VersionMiddleware::new(inner)
    }
}

impl<S> Service<hyper::Request<Body>> for VersionMiddleware<S>
where
    S: Service<hyper::Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();

        Box::pin(async move {
            match add_rpc_version_to_method(&mut req).await {
                Ok(()) => inner.call(req).await,
                Err(e) => {
                    let error = match e {
                        VersionMiddlewareError::InvalidUrlFormat => {
                            ErrorObject::owned(-32600, "Invalid URL format. Use /rpc/v{version}", None::<()>)
                        }
                        VersionMiddlewareError::InvalidVersion => {
                            ErrorObject::owned(-32600, "Invalid RPC version specified", None::<()>)
                        }
                        VersionMiddlewareError::InvalidRequestFormat => {
                            ErrorObject::owned(-32600, "Invalid JSON-RPC request format", None::<()>)
                        }
                        VersionMiddlewareError::UnsupportedVersion => {
                            ErrorObject::owned(-32601, "Unsupported RPC version specified", None::<()>)
                        }
                        _ => ErrorObject::owned(-32603, "Internal error", None::<()>),
                    };

                    let body = json!({
                        "jsonrpc": "2.0",
                        "error": error,
                        "id": 0
                    })
                    .to_string();

                    Ok(Response::builder()
                        .header("Content-Type", "application/json")
                        .body(Body::from(body))
                        .unwrap_or_else(|_| Response::new(Body::from("Internal server error"))))
                }
            }
        })
    }
}

async fn add_rpc_version_to_method(req: &mut hyper::Request<Body>) -> Result<(), VersionMiddlewareError> {
    let path = req.uri().path().to_string();
    let version = RpcVersion::from_request_path(&path)?;

    let whole_body = hyper::body::to_bytes(req.body_mut()).await?;
    let mut json: Value = serde_json::from_slice(&whole_body)?;

    if let Some(method) = json.get_mut("method").as_deref().and_then(Value::as_str) {
        let new_method = format!("starknet_{}_{}", version.name(), method.strip_prefix("starknet_").unwrap_or(method));

        json["method"] = Value::String(new_method);
    } else {
        return Err(VersionMiddlewareError::InvalidRequestFormat);
    }

    let new_body = Body::from(serde_json::to_vec(&json)?);
    *req.body_mut() = new_body;

    Ok(())
}
