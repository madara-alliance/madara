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
use jsonrpsee::server::ws;
use jsonrpsee::types::ErrorObject;
use jsonrpsee::MethodResponse;
use serde_json::{json, Value};
use tower::Service;

use mp_chain_config::{RpcVersion, RpcVersionError};

pub use super::metrics::Metrics;

/// Rate limit middleware
#[derive(Clone, Debug)]
pub struct RateLimit {
    pub(crate) limiter: Arc<governor::RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>,
    pub(crate) clock: QuantaClock,
}

impl RateLimit {
    pub fn new(max_burst: NonZeroU32) -> Self {
        let clock = QuantaClock::default();
        Self { limiter: Arc::new(RateLimiter::direct_with_clock(Quota::per_minute(max_burst), &clock)), clock }
    }
}

const MAX_JITTER: Duration = Duration::from_millis(50);
const MAX_RETRIES: usize = 10;

#[derive(Clone, Debug)]
pub struct RpcLayerRateLimit {
    rate_limit: RateLimit,
}

impl RpcLayerRateLimit {
    pub fn new(n: NonZeroU32) -> Self {
        Self { rate_limit: RateLimit::new(n) }
    }
}

impl<S> tower::Layer<S> for RpcLayerRateLimit {
    type Service = RpcMiddleWareServiceRateLimit<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcMiddleWareServiceRateLimit { inner, rate_limit: self.rate_limit.clone() }
    }
}

#[derive(Clone, Debug)]
pub struct RpcLayerMetrics {
    metrics: Metrics,
}

impl RpcLayerMetrics {
    /// Enable metrics middleware.
    pub fn new(metrics: Metrics) -> Self {
        Self { metrics }
    }

    /// Register a new websocket connection.
    pub fn ws_connect(&self) {
        self.metrics.ws_connect()
    }

    /// Register that a websocket connection was closed.
    pub fn ws_disconnect(&self, now: Instant) {
        self.metrics.ws_disconnect(now)
    }
}

impl<S> tower::Layer<S> for RpcLayerMetrics {
    type Service = RpcMiddleWareServiceMetrics<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcMiddleWareServiceMetrics { inner, metrics: self.metrics.clone() }
    }
}

#[derive(Clone, Debug)]
pub struct RpcMiddleWareServiceRateLimit<S> {
    inner: S,
    rate_limit: RateLimit,
}

impl<'a, S> RpcServiceT<'a> for RpcMiddleWareServiceRateLimit<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = BoxFuture<'a, MethodResponse>;

    fn call(&self, mut req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let rate_limit = self.rate_limit.clone();

        async move {
            let mut attempts = 0;
            let jitter = Jitter::up_to(MAX_JITTER);
            let mut rate_limited = false;

            loop {
                if attempts >= MAX_RETRIES {
                    return MethodResponse::error(
                        req.id,
                        ErrorObject::owned(-32999, "RPC rate limit exceeded", None::<()>),
                    );
                }

                if let Err(rejected) = rate_limit.limiter.check() {
                    tokio::time::sleep(jitter + rejected.wait_time_from(rate_limit.clock.now())).await;
                    rate_limited = true;
                } else {
                    break;
                }

                attempts += 1;
            }

            // This should be ok as a way to flag rate limited requests as the
            // JSON RPC spec discourages the use of NULL as an id in a _request_
            // since it is used for_responses_ with an unknown id.
            if rate_limited == true {
                req.id = jsonrpsee::types::Id::Null;
            }

            inner.call(req).await
        }
        .boxed()
    }
}

#[derive(Clone, Debug)]
pub struct RpcMiddleWareServiceMetrics<S> {
    inner: S,
    metrics: Metrics,
}

impl<'a, S> RpcServiceT<'a> for RpcMiddleWareServiceMetrics<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = BoxFuture<'a, MethodResponse>;

    fn call(&self, mut req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();
        metrics.on_call(&req);

        async move {
            let is_rate_limited = if matches!(req.id, jsonrpsee::types::params::Id::Null) {
                req.id = jsonrpsee::types::params::Id::Number(1);
                true
            } else {
                false
            };

            let now = std::time::Instant::now();

            let rp = inner.call(req.clone()).await;

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

            metrics.on_response(&req, &rp, is_rate_limited, now);

            rp
        }
        .boxed()
    }
}

#[derive(Clone, Debug)]
pub struct RpcMiddlewareServiceVerions<S> {
    inner: S,
}

#[derive(thiserror::Error, Debug)]
enum HttpMiddlewareServiceVersionError {
    #[error("Failed to read request body: {0}")]
    BodyReadError(#[from] hyper::Error),
    #[error("Failed to parse JSON: {0}")]
    JsonParseError(#[from] serde_json::Error),
    #[error("Invalid URL format")]
    InvalidUrlFormat,
    #[error("Invalid version specified")]
    InvalidVersion,
    #[error("Unsupported version specified")]
    UnsupportedVersion,
}

impl From<RpcVersionError> for HttpMiddlewareServiceVersionError {
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

#[derive(Clone)]
pub struct VersionMiddlewareLayer;

impl<S> tower::Layer<S> for VersionMiddlewareLayer {
    type Service = RpcMiddlewareServiceVerions<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcMiddlewareServiceVerions::new(inner)
    }
}

impl<S> RpcMiddlewareServiceVerions<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<'a, S> RpcServiceT<'a> for RpcMiddlewareServiceVerions<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = S::Future;

    fn call(&self, req: jsonrpsee::types::Request<'a>) -> Self::Future {
        self.inner.call(req)
    }
}

// impl<S> Service<hyper::Request<Body>> for RpcMiddlewareServiceVerion<S>
// where
//     S: Service<hyper::Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
//     S::Future: Send + 'static,
// {
//     type Response = S::Response;
//     type Error = S::Error;
//     type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;
//
//     fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
//         self.inner.poll_ready(cx)
//     }
//
//     fn call(&mut self, mut req: hyper::Request<Body>) -> Self::Future {
//         let mut inner = self.inner.clone();
//
//         Box::pin(async move {
//             match add_rpc_version_to_method(&mut req).await {
//                 Ok(()) => inner.call(req).await,
//                 Err(e) => {
//                     let error = match e {
//                         HttpMiddlewareServiceVersionError::InvalidUrlFormat => {
//                             ErrorObject::owned(-32600, "Invalid URL format. Use /rpc/v{version}", None::<()>)
//                         }
//                         HttpMiddlewareServiceVersionError::InvalidVersion => {
//                             ErrorObject::owned(-32600, "Invalid RPC version specified", None::<()>)
//                         }
//                         HttpMiddlewareServiceVersionError::UnsupportedVersion => {
//                             ErrorObject::owned(-32601, "Unsupported RPC version specified", None::<()>)
//                         }
//                         _ => ErrorObject::owned(-32603, "Internal error", None::<()>),
//                     };
//
//                     let body = json!({
//                         "jsonrpc": "2.0",
//                         "error": error,
//                         "id": null
//                     })
//                     .to_string();
//
//                     Ok(Response::builder()
//                         .header("Content-Type", "application/json")
//                         .body(Body::from(body))
//                         .unwrap_or_else(|_| Response::new(Body::from("Internal server error"))))
//                 }
//             }
//         })
//     }
// }

async fn add_rpc_version_to_method(req: &mut hyper::Request<Body>) -> Result<(), HttpMiddlewareServiceVersionError> {
    if ws::is_upgrade_request(req) {
        log::debug!(target: "rpc_version", "request version not mutated on websocket connection calls");
        return Ok(());
    }

    log::debug!(target: "rpc_version", "adding rpc version call");

    let path = req.uri().path().to_string();
    let version = RpcVersion::from_request_path(&path)?;

    log::debug!(target: "rpc_version", "found version {version}");

    let whole_body = hyper::body::to_bytes(req.body_mut()).await?;
    let json: Value = serde_json::from_slice(&whole_body)?;

    log::trace!(target: "rpc_version", "request body is {}", serde_json::to_string_pretty(&json).unwrap_or_default());

    // in case of batched requests, the request is an array of JSON-RPC requests
    let mut batched_request = false;
    let mut items = if let Value::Array(items) = json {
        log::debug!(target: "rpc_version", "request is batched");
        batched_request = true;
        items
    } else {
        log::debug!(target: "rpc_version", "request is not batched");
        vec![json]
    };

    for item in items.iter_mut() {
        if let Some(method) = item.get_mut("method").as_deref().and_then(Value::as_str) {
            log::debug!(target: "rpc_version", "resolving method: {method}");
            let new_method =
                format!("starknet_{}_{}", version.name(), method.strip_prefix("starknet_").unwrap_or(method));
            log::debug!(target: "rpc_version", "added version to method: {new_method}");

            item["method"] = Value::String(new_method);
        }
        // we don't need to throw an error here, the request will be rejected later if the method is not supported
    }

    let response = if batched_request { serde_json::to_vec(&items)? } else { serde_json::to_vec(&items[0])? };
    *req.body_mut() = Body::from(response);

    Ok(())
}
