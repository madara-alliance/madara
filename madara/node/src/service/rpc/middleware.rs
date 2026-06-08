//! JSON-RPC specific middleware.

use anyhow::Context;
use jsonrpsee::server::middleware::rpc::{Batch, MethodResponse, Notification, Request, RpcServiceT};
use mp_chain_config::RpcVersion;
use std::time::Instant;

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

impl<S> RpcServiceT for RpcMiddlewareServiceMetrics<S>
where
    S: Send
        + Sync
        + Clone
        + RpcServiceT<
            MethodResponse = MethodResponse,
            BatchResponse = MethodResponse,
            NotificationResponse = MethodResponse,
        > + 'static,
{
    type MethodResponse = MethodResponse;
    type NotificationResponse = MethodResponse;
    type BatchResponse = MethodResponse;

    fn call<'a>(&self, req: Request<'a>) -> impl std::future::Future<Output = Self::MethodResponse> + Send + 'a {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();

        async move {
            let now = std::time::Instant::now();

            tracing::trace!(
                target: "rpc_raw_request",
                "{:?}",
                req.params().as_str()
            );

            metrics.on_call(&req);
            let rp = inner.call(req.clone()).await;

            let method = req.method_name();
            let status = rp.as_error_code().unwrap_or(200) as i64;
            let res_len = rp.as_json().get().len() as u64;
            let response_time = now.elapsed().as_micros();

            tracing::info!(
                target: "rpc_calls",
                method = method,
                status = status,
                res_len = res_len,
                response_time = response_time,
                "{method} {status} {res_len} - {response_time} micros"
            );

            tracing::trace!(
                target: "rpc_raw_response",
                "{:?}",
                rp.as_json().get()
            );

            metrics.on_response(&req, &rp, now);

            rp
        }
    }

    fn batch<'a>(&self, requests: Batch<'a>) -> impl std::future::Future<Output = Self::BatchResponse> + Send + 'a {
        let inner = self.inner.clone();
        async move { inner.batch(requests).await }
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl std::future::Future<Output = Self::NotificationResponse> + Send + 'a {
        let inner = self.inner.clone();
        async move { inner.notification(n).await }
    }
}

#[derive(Debug, Clone)]
pub struct RpcMiddlewareServiceVersion<S> {
    inner: S,
    path: String,
    version_default: RpcVersion,
}

impl<S> RpcMiddlewareServiceVersion<S> {
    pub fn new(inner: S, path: String, version_default: RpcVersion) -> Self {
        Self { inner, path, version_default }
    }
}

impl<S> RpcServiceT for RpcMiddlewareServiceVersion<S>
where
    S: Send
        + Sync
        + Clone
        + RpcServiceT<
            MethodResponse = MethodResponse,
            BatchResponse = MethodResponse,
            NotificationResponse = MethodResponse,
        > + 'static,
{
    type MethodResponse = MethodResponse;
    type NotificationResponse = MethodResponse;
    type BatchResponse = MethodResponse;

    fn call<'a>(&self, mut req: Request<'a>) -> impl std::future::Future<Output = Self::MethodResponse> + Send + 'a {
        let inner = self.inner.clone();
        let path = self.path.clone();
        let version_default = self.version_default;

        async move {
            if req.method == "rpc_methods" {
                return inner.call(req).await;
            }

            let version = match RpcVersion::from_request_path(&path, version_default)
                .map(|v| v.name())
                .context("Failed to get request path")
            {
                Ok(version) => version,
                Err(_) => {
                    return jsonrpsee::MethodResponse::error(
                        req.id,
                        jsonrpsee::types::ErrorObject::owned(
                            jsonrpsee::types::error::PARSE_ERROR_CODE,
                            jsonrpsee::types::error::PARSE_ERROR_MSG,
                            None::<()>,
                        ),
                    )
                }
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

            let method = method.replacen(&format!("{version}_"), "", 1);
            let method_new = format!("{namespace}_{version}_{method}");
            req.method = jsonrpsee::core::Cow::from(method_new);

            inner.call(req).await
        }
    }

    fn batch<'a>(&self, requests: Batch<'a>) -> impl std::future::Future<Output = Self::BatchResponse> + Send + 'a {
        let inner = self.inner.clone();
        async move { inner.batch(requests).await }
    }

    fn notification<'a>(
        &self,
        n: Notification<'a>,
    ) -> impl std::future::Future<Output = Self::NotificationResponse> + Send + 'a {
        let inner = self.inner.clone();
        async move { inner.notification(n).await }
    }
}
