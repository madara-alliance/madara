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
use jsonrpsee::types::{ErrorObject, Request};
use jsonrpsee::MethodResponse;

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
            if let Some(m) = metrics.as_ref() {
                m.on_response(&req, &rp, is_rate_limited, now)
            }

            rp
        }
        .boxed()
    }
}
