//! Own admitted RPC calls independently of HTTP/WS connections and drain them before service stop.
use futures::{future::BoxFuture, FutureExt};
use jsonrpsee::{server::middleware::rpc::RpcServiceT, types::Request, MethodResponse};
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub(super) struct TrackedRpcRequests<S> {
    inner: S,
    requests: TaskTracker,
}

impl<S> TrackedRpcRequests<S> {
    pub(super) fn new(inner: S, requests: TaskTracker) -> Self {
        Self { inner, requests }
    }
}

impl<'a, S> RpcServiceT<'a> for TrackedRpcRequests<S>
where
    S: Send + Sync + Clone + 'static + for<'request> RpcServiceT<'request>,
{
    type Future = BoxFuture<'a, MethodResponse>;

    fn call(&self, request: Request<'a>) -> Self::Future {
        // Register before checking closure: shutdown either waits for this call or it is
        // rejected. TaskTracker::close alone does not prevent admission on an existing WS.
        let token = self.requests.token();
        if self.requests.is_closed() {
            return futures::future::ready(MethodResponse::error(
                request.id,
                jsonrpsee::types::ErrorObject::owned(-32000, "RPC server is stopping", None::<()>),
            ))
            .boxed();
        }
        let inner = self.inner.clone();
        let request = Request {
            jsonrpc: request.jsonrpc,
            id: request.id.into_owned(),
            method: jsonrpsee::core::Cow::owned(request.method.into_owned()),
            params: request.params.map(|params| std::borrow::Cow::Owned(params.into_owned())),
        };
        let id = request.id.clone();
        let task = tokio::spawn(async move {
            let _token = token;
            inner.call(request).await
        });
        async move {
            task.await.unwrap_or_else(|_| MethodResponse::error(id, jsonrpsee::types::error::ErrorCode::InternalError))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
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
        let service = TrackedRpcRequests::new(
            BlockingRead {
                started: Arc::new(Mutex::new(Some(started))),
                release: Arc::new(Mutex::new(receiver)),
                finished: Arc::clone(&finished),
            },
            requests.clone(),
        );
        let request = || Request::new("historicalProof".into(), None, jsonrpsee::types::Id::Number(1));
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
