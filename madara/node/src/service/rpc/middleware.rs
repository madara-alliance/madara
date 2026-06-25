//! JSON-RPC specific middleware.

use jsonrpsee::{
    server::middleware::rpc::{Batch, BatchEntry, BatchEntryErr, Notification, RpcServiceT},
    MethodResponse,
};
use mp_chain_config::RpcVersion;
use std::future::Future;
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
            NotificationResponse = MethodResponse,
            BatchResponse = MethodResponse,
        > + 'static,
{
    type MethodResponse = MethodResponse;
    type NotificationResponse = MethodResponse;
    type BatchResponse = MethodResponse;

    fn call<'a>(&self, req: jsonrpsee::types::Request<'a>) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
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
                rp.as_json()
            );

            metrics.on_response(&req, &rp, now);

            rp
        }
    }

    fn batch<'a>(&self, batch: Batch<'a>) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();

        async move {
            let now = std::time::Instant::now();
            let methods = batch
                .iter()
                .filter_map(|entry| entry.as_ref().ok().map(BatchEntry::method_name))
                .map(str::to_owned)
                .collect::<Vec<_>>();

            for method in &methods {
                metrics.inner.on_call_method(method);
            }

            let rp = inner.batch(batch).await;

            for method in &methods {
                metrics.inner.on_response_method(method, rp.is_success(), now);
            }

            rp
        }
    }

    fn notification<'a>(
        &self,
        notification: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        let inner = self.inner.clone();
        let metrics = self.metrics.clone();

        async move {
            let now = std::time::Instant::now();
            let method = notification.method_name().to_owned();

            metrics.inner.on_call_method(&method);
            let rp = inner.notification(notification).await;
            metrics.inner.on_response_method(&method, rp.is_success(), now);

            rp
        }
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
            NotificationResponse = MethodResponse,
            BatchResponse = MethodResponse,
        > + 'static,
{
    type MethodResponse = MethodResponse;
    type NotificationResponse = MethodResponse;
    type BatchResponse = MethodResponse;

    fn call<'a>(
        &self,
        mut req: jsonrpsee::types::Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let inner = self.inner.clone();
        let path = self.path.clone();
        let version_default = self.version_default;

        async move {
            if let Err(error) = rewrite_method(&mut req.method, &path, version_default) {
                return MethodResponse::error(req.id(), error);
            }

            inner.call(req).await
        }
    }

    fn batch<'a>(&self, mut batch: Batch<'a>) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        let inner = self.inner.clone();
        let path = self.path.clone();
        let version_default = self.version_default;

        async move {
            for batch_entry in batch.iter_mut() {
                let error = match batch_entry {
                    Ok(entry) => match entry {
                        BatchEntry::Call(req) => {
                            rewrite_method(&mut req.method, &path, version_default).err().map(|error| (req.id(), error))
                        }
                        BatchEntry::Notification(notification) => {
                            let _ = rewrite_method(&mut notification.method, &path, version_default);
                            None
                        }
                    },
                    Err(_) => None,
                };

                if let Some((id, error)) = error {
                    *batch_entry = Err(BatchEntryErr::new(id, error));
                }
            }

            inner.batch(batch).await
        }
    }

    fn notification<'a>(
        &self,
        mut notification: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        let inner = self.inner.clone();
        let path = self.path.clone();
        let version_default = self.version_default;

        async move {
            let _ = rewrite_method(&mut notification.method, &path, version_default);
            inner.notification(notification).await
        }
    }
}

fn rewrite_method(
    method: &mut jsonrpsee::core::Cow<'_, str>,
    path: &str,
    version_default: RpcVersion,
) -> Result<(), jsonrpsee::types::ErrorObject<'static>> {
    if method.as_ref() == "rpc_methods" {
        return Ok(());
    }

    let version = RpcVersion::from_request_path(path, version_default).map(|v| v.name()).map_err(|_| {
        jsonrpsee::types::ErrorObject::owned(
            jsonrpsee::types::error::PARSE_ERROR_CODE,
            jsonrpsee::types::error::PARSE_ERROR_MSG,
            None::<()>,
        )
    })?;

    let Some((namespace, method_name)) = method.split_once('_') else {
        return Err(jsonrpsee::types::ErrorObject::owned(
            jsonrpsee::types::error::METHOD_NOT_FOUND_CODE,
            jsonrpsee::types::error::METHOD_NOT_FOUND_MSG,
            Some(method.to_string()),
        ));
    };

    let method_name = method_name.replacen(&format!("{version}_"), "", 1);
    *method = jsonrpsee::core::Cow::from(format!("{namespace}_{version}_{method_name}"));

    Ok(())
}
