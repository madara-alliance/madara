//! JSON-RPC specific middleware.

use anyhow::Context;
use futures::future::{BoxFuture, FutureExt};
use jsonrpsee::{server::middleware::rpc::RpcServiceT, types::Request, MethodResponse};
use mp_chain_config::RpcVersion;
use std::time::Instant;
use tokio_util::task::TaskTracker;

pub use super::metrics::Metrics;

fn trace_rpc_body(method: &str) -> bool {
    !method.contains("CommittedData")
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

            // Private datasets and witness requests must not enter raw RPC parameter logs.
            // Both import methods carry raw dataset values, including the combined
            // import-and-submit method. Skip traces for every committed-data RPC.
            let trace_body = trace_rpc_body(&req.method);
            if trace_body {
                tracing::trace!(target: "rpc_raw_request", "{:?}", req.params().as_str());
            }

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

            if trace_body {
                tracing::trace!(target: "rpc_raw_response", "{:?}", rp.as_result());
            }

            metrics.on_response(&req, &rp, now);

            rp
        }
        .boxed()
    }
}

#[cfg(test)]
mod committed_data_tests {
    use super::trace_rpc_body;

    #[test]
    fn raw_traces_exclude_both_dataset_import_methods() {
        assert!(!trace_rpc_body("madara_V0_1_0_importCommittedDataSet"));
        assert!(!trace_rpc_body("madara_V0_1_0_importCommittedDataAndSubmitTransaction"));
        assert!(!trace_rpc_body("madara_V0_1_0_getCommittedDataWitness"));
        assert!(trace_rpc_body("starknet_V0_10_2_getBlockWithTxHashes"));
    }
}

/// Resolve the RPC version and own admitted calls until completion, even after disconnect.
#[derive(Debug, Clone)]
pub struct RpcDispatcher<S> {
    inner: S,
    path: String,
    version_default: RpcVersion,
    requests: TaskTracker,
}

impl<S> RpcDispatcher<S> {
    pub fn new(inner: S, path: String, version_default: RpcVersion, requests: TaskTracker) -> Self {
        Self { inner, path, version_default, requests }
    }
}

impl<'a, S> RpcServiceT<'a> for RpcDispatcher<S>
where
    S: Send + Sync + Clone + 'static + for<'request> RpcServiceT<'request>,
{
    type Future = BoxFuture<'a, jsonrpsee::MethodResponse>;

    fn call(&self, req: Request<'a>) -> Self::Future {
        // Register before checking closure so service stop either waits for this call
        // or rejects it. Cancellation cannot stop a running blocking DB operation.
        let token = self.requests.token();
        if self.requests.is_closed() {
            return futures::future::ready(MethodResponse::error(
                req.id,
                jsonrpsee::types::ErrorObject::owned(-32000, "RPC server is stopping", None::<()>),
            ))
            .boxed();
        }
        let mut req = Request {
            jsonrpc: req.jsonrpc,
            id: req.id.into_owned(),
            method: jsonrpsee::core::Cow::owned(req.method.into_owned()),
            params: req.params.map(|params| std::borrow::Cow::Owned(params.into_owned())),
        };
        let id = req.id.clone();
        let inner = self.inner.clone();
        let path = self.path.clone();
        let version_default = self.version_default;

        let task = tokio::spawn(async move {
            let _token = token;
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
        });
        async move {
            task.await.unwrap_or_else(|_| MethodResponse::error(id, jsonrpsee::types::error::ErrorCode::InternalError))
        }
        .boxed()
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;

    /// Models the blocking tree rebuild used by both HTTP and WS method dispatch. The
    /// response owner may disappear, but service shutdown must still await the DB writer.
    #[derive(Clone)]
    struct BlockingRead {
        started: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        release: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
        finished: Arc<AtomicBool>,
    }

    impl<'a> RpcServiceT<'a> for BlockingRead {
        type Future = BoxFuture<'a, MethodResponse>;
        fn call(&self, request: Request<'a>) -> Self::Future {
            assert_eq!(request.method_name(), "starknet_V0_10_2_historicalProof");
            let read = self.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    read.started.lock().unwrap().take().unwrap().send(()).unwrap();
                    let _ = read.release.lock().unwrap().recv_timeout(Duration::from_secs(10));
                    read.finished.store(true, Ordering::SeqCst);
                })
                .await
                .unwrap();
                MethodResponse::error(request.id, jsonrpsee::types::error::ErrorCode::InternalError)
            }
            .boxed()
        }
    }

    #[rstest::rstest]
    #[case::disconnect(true)]
    #[case::connected(false)]
    #[tokio::test]
    async fn shutdown_drains_admitted_blocking_calls_and_rejects_new_ones(#[case] disconnect: bool) {
        let requests = TaskTracker::new();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, receiver) = std::sync::mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let service = RpcDispatcher::new(
            BlockingRead {
                started: Arc::new(Mutex::new(Some(started))),
                release: Arc::new(Mutex::new(receiver)),
                finished: Arc::clone(&finished),
            },
            "/rpc/v0_10_2/".into(),
            RpcVersion::RPC_VERSION_0_10_2,
            requests.clone(),
        );
        let request = || Request::new("starknet_historicalProof".into(), None, jsonrpsee::types::Id::Number(1));
        let response = service.call(request());
        ready.await.unwrap();
        let response = if disconnect {
            drop(response);
            None
        } else {
            Some(response)
        };
        requests.close();
        assert!(service.call(request()).await.as_result().contains("RPC server is stopping"));
        assert!(tokio::time::timeout(Duration::from_millis(20), requests.wait()).await.is_err());
        assert!(!finished.load(Ordering::SeqCst));
        drop(release);
        tokio::time::timeout(Duration::from_secs(1), requests.wait()).await.unwrap();
        assert!(finished.load(Ordering::SeqCst));
        if let Some(response) = response {
            response.await;
        }
    }
}
