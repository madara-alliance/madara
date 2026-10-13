//! JSON-RPC specific middleware.

use anyhow::Context;
use futures::future::{BoxFuture, FutureExt};
use jsonrpsee::server::middleware::rpc::RpcServiceT;
use mp_chain_config::RpcVersion;
use std::{sync::Arc, time::Instant};
use tokio::sync::Semaphore;

pub use super::metrics::Metrics;

#[derive(Debug, Clone)]
pub struct RpcExecutionLimiterLayer {
    semaphore: Arc<Semaphore>,
}

impl RpcExecutionLimiterLayer {
    pub fn new(semaphore: Arc<Semaphore>) -> Self {
        Self { semaphore }
    }
}

impl<S> tower::Layer<S> for RpcExecutionLimiterLayer {
    type Service = RpcExecutionLimiterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RpcExecutionLimiterService { inner, semaphore: Arc::clone(&self.semaphore) }
    }
}

#[derive(Debug, Clone)]
pub struct RpcExecutionLimiterService<S> {
    inner: S,
    semaphore: Arc<Semaphore>,
}

fn is_execution_method(method: &str) -> bool {
    const EXECUTION_METHODS: &[&str] = &[
        "call",
        "estimateFee",
        "estimateMessageFee",
        "simulateTransactions",
        "traceTransaction",
        "traceBlockTransactions",
        "addInvokeTransaction",
        "addDeclareTransaction",
        "addDeployAccountTransaction",
        "addDeclareV0Transaction",
    ];

    EXECUTION_METHODS.contains(&method.rsplit('_').next().unwrap_or(method))
}

impl<'a, S> RpcServiceT<'a> for RpcExecutionLimiterService<S>
where
    S: Send + Sync + Clone + RpcServiceT<'a> + 'static,
{
    type Future = BoxFuture<'a, jsonrpsee::MethodResponse>;

    fn call(&self, req: jsonrpsee::types::Request<'a>) -> Self::Future {
        let inner = self.inner.clone();

        if !is_execution_method(req.method_name()) {
            return inner.call(req).boxed();
        }

        let semaphore = Arc::clone(&self.semaphore);
        async move {
            let _permit = semaphore.acquire_owned().await.expect("RPC execution semaphore should remain open");
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

    fn call(&self, req: jsonrpsee::types::Request<'a>) -> Self::Future {
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
            let res_len = rp.as_result().len() as u64;
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
                rp.as_result()
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
    version_default: RpcVersion,
}

impl<S> RpcMiddlewareServiceVersion<S> {
    pub fn new(inner: S, path: String, version_default: RpcVersion) -> Self {
        Self { inner, path, version_default }
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
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::is_execution_method;

    #[test]
    fn execution_method_matching_covers_public_and_versioned_names() {
        for method in [
            "starknet_call",
            "starknet_V0_10_0_call",
            "starknet_estimateFee",
            "starknet_V0_9_0_simulateTransactions",
            "starknet_traceBlockTransactions",
            "starknet_addInvokeTransaction",
            "madara_addDeclareV0Transaction",
        ] {
            assert!(is_execution_method(method), "{method} should be limited");
        }

        for method in ["starknet_blockNumber", "starknet_getStorageAt", "rpc_methods"] {
            assert!(!is_execution_method(method), "{method} should not be limited");
        }
    }
}
