//! JSON-RPC specific middleware.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::{BoxFuture, FutureExt};
use governor::clock::{Clock, DefaultClock, QuantaClock};
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter};
use jsonrpsee::server::middleware::rpc::RpcServiceT;

use mp_chain_config::RpcVersion;

pub use super::metrics::Metrics;

/// Rate limit middleware
#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct RpcMiddlewareLayerRateLimit {
    rate_limit: RateLimit,
}

impl RpcMiddlewareLayerRateLimit {
    pub fn new(n: NonZeroU32) -> Self {
        Self { rate_limit: RateLimit::new(n) }
    }
}

impl<S> tower::Layer<S> for RpcMiddlewareLayerRateLimit {
    type Service = RpcMiddlewareServiceRateLimit<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcMiddlewareServiceRateLimit { inner, rate_limit: self.rate_limit.clone() }
    }
}

#[derive(Debug, Clone)]
pub struct RpcMiddlewareServiceRateLimit<S> {
    inner: S,
    rate_limit: RateLimit,
}

impl<'a, S> RpcServiceT<'a> for RpcMiddlewareServiceRateLimit<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = BoxFuture<'a, jsonrpsee::MethodResponse>;

    fn call(&self, mut req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let rate_limit = self.rate_limit.clone();

        async move {
            let mut attempts = 0;
            let jitter = Jitter::up_to(MAX_JITTER);
            let mut rate_limited = false;

            loop {
                if attempts >= MAX_RETRIES {
                    return jsonrpsee::MethodResponse::error(
                        req.id,
                        jsonrpsee::types::ErrorObject::owned(-32099, "RPC rate limit exceeded", None::<()>),
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
            // since it is used for _responses_ with an unknown id.
            if rate_limited {
                req.id = jsonrpsee::types::Id::Null;
            }

            inner.call(req).await
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
pub struct RpcMiddlewareLayerMetrics {
    metrics: Metrics,
}

impl RpcMiddlewareLayerMetrics {
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

impl<S> tower::Layer<S> for RpcMiddlewareLayerMetrics {
    type Service = RpcMiddlewareServiceMetrics<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcMiddlewareServiceMetrics { inner, metrics: self.metrics.clone() }
    }
}

#[derive(Debug, Clone)]
pub struct RpcMiddlewareServiceMetrics<S> {
    inner: S,
    metrics: Metrics,
}

impl<'a, S> RpcServiceT<'a> for RpcMiddlewareServiceMetrics<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = BoxFuture<'a, jsonrpsee::MethodResponse>;

    fn call(&self, mut req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();

        async move {
            let is_rate_limited = if matches!(req.id, jsonrpsee::types::params::Id::Null) {
                req.id = jsonrpsee::types::params::Id::Number(1);
                true
            } else {
                false
            };

            let now = std::time::Instant::now();

            metrics.on_call(&req);
            let rp = inner.call(req.clone()).await;

            let method = req.method_name();
            let status = rp.as_error_code().unwrap_or(200);
            let res_len = rp.as_result().len();
            let response_time = now.elapsed();

            tracing::info!(
                target: "rpc_calls",
                method = method,
                status = status,
                res_len = res_len,
                response_time = response_time.as_micros(),
                "{method} {status} {res_len} - {response_time:?}",
            );

            metrics.on_response(&req, &rp, is_rate_limited, now);

            rp
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
pub struct RpcMiddlewareServiceVersion<S> {
    inner: S,
    path: String,
}

impl<S> RpcMiddlewareServiceVersion<S> {
    pub fn new(inner: S, path: String) -> Self {
        Self { inner, path }
    }
}

impl<'a, S> RpcServiceT<'a> for RpcMiddlewareServiceVersion<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = BoxFuture<'a, jsonrpsee::MethodResponse>;

    fn call(&self, mut req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let path = self.path.clone();

        async move {
            if req.method == "rpc_methods" {
                return inner.call(req).await;
            }

            let Ok(version) = RpcVersion::from_request_path(&path).map(|v| v.name()) else {
                return jsonrpsee::MethodResponse::error(
                    req.id,
                    jsonrpsee::types::ErrorObject::owned(
                        jsonrpsee::types::error::PARSE_ERROR_CODE,
                        jsonrpsee::types::error::PARSE_ERROR_MSG,
                        None::<()>,
                    ),
                );
            };

            let Some((namespace, method)) = req.method.split_once('_') else {
                return jsonrpsee::MethodResponse::error(
                    req.id(),
                    jsonrpsee::types::ErrorObject::owned(
                        jsonrpsee::types::error::METHOD_NOT_FOUND_CODE,
                        jsonrpsee::types::error::METHOD_NOT_FOUND_MSG,
                        Some(req.method_name()),
                    ),
                );
            };

            let method_new = format!("{namespace}_{version}_{method}");
            req.method = jsonrpsee::core::Cow::from(method_new);

            inner.call(req).await
        }
        .boxed()
    }
}
