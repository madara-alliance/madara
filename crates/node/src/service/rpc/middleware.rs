//! JSON-RPC specific middleware.

use std::time::Instant;

use futures::future::{BoxFuture, FutureExt};
use jsonrpsee::server::middleware::rpc::RpcServiceT;

use mp_chain_config::RpcVersion;

pub use super::metrics::Metrics;

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

    fn call(&self, req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();

        async move {
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

            metrics.on_response(&req, &rp, now);

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
