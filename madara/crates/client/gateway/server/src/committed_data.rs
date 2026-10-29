//! Dataset replication and witness retrieval. Reads may renew/rebuild the local proof index;
//! this interface never ingests new datasets.
use crate::{
    error::GatewayError,
    helpers::{create_json_response, get_params_from_request},
};
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use http_body_util::{BodyExt, Limited};
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use mc_db::{MadaraBackend, MadaraStorageRead};
use mp_gateway::{
    committed_data::{CommittedDataPage, CommittedDataVersion, VALUES_PER_PAGE},
    error::{StarknetError, StarknetErrorCode},
};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Default per-server budget for blocking replication/proof reads.
pub const DEFAULT_MAX_CONCURRENT_READS: usize = 4;

fn invalid() -> GatewayError {
    StarknetError::new(StarknetErrorCode::MalformedRequest, "Invalid committed-data query".into()).into()
}

/// Keep the permit in the blocking task even if the HTTP client disconnects.
async fn bounded_read(
    reads: Arc<Semaphore>,
    read: impl FnOnce() -> Result<Response<String>, GatewayError> + Send + 'static,
) -> Result<Response<String>, GatewayError> {
    let permit = reads.try_acquire_owned().map_err(|_| GatewayError::from(StarknetError::rate_limited()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        read()
    })
    .await
    .map_err(|e| GatewayError::from(anyhow::anyhow!(e)))?
}

/// Enumerate complete datasets; the cursor is lexical and consumers rescan.
pub(crate) async fn get_roots(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    reads: Arc<Semaphore>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let after = params.get("after").map(|value| Felt::from_hex(value).map_err(|_| invalid())).transpose()?;
    bounded_read(reads, move || Ok(create_json_response(StatusCode::OK, &backend.db.list_committed_data_roots(after)?)))
        .await
}

/// Replicate namespace labels. The configured upstream is trusted for labels, never root publication.
/// A cursor supplies all three components or none, ordered application/dataset/root.
pub(crate) async fn get_versions(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    reads: Arc<Semaphore>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let parse = |name| params.get(name).map(|value| Felt::from_hex(value).map_err(|_| invalid())).transpose();
    let after = match (parse("application_id")?, parse("dataset_id")?, parse("root")?) {
        (None, None, None) => None,
        (Some(application_id), Some(dataset_id), Some(root)) => {
            Some(CommittedDataVersion { application_id, dataset_id, root })
        }
        _ => return Err(invalid()),
    };
    bounded_read(reads, move || {
        Ok(create_json_response(StatusCode::OK, &backend.db.list_committed_data_versions(after)?))
    })
    .await
}

/// Return one bounded value slice; consumers authenticate the assembled root.
pub(crate) async fn get_page(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    reads: Arc<Semaphore>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let root = params.get("root").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
    let start = params.get("start").map(|value| value.parse::<u32>().map_err(|_| invalid())).transpose()?.unwrap_or(0);
    if (start as usize) % VALUES_PER_PAGE != 0 || (start as usize) >= MAX_COMMITTED_DATA_VALUES {
        return Err(invalid());
    }
    bounded_read(reads, move || {
        let page = backend.db.get_committed_data_page(root, start)?.map(|(count, values)| CommittedDataPage {
            root,
            start,
            count,
            entries: values,
        });
        Ok(create_json_response(StatusCode::OK, &page))
    })
    .await
}

/// Return a keyed Patricia witness used by the proving pipeline.
pub(crate) async fn get_witness(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    reads: Arc<Semaphore>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let root = params.get("root").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
    let index = params.get("key").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
    bounded_read(reads, move || Ok(create_json_response(StatusCode::OK, &backend.committed_data_witness(root, index)?)))
        .await
}

/// Bounded read-only witness transport. Authentication remains mandatory at the receiver.
pub(crate) async fn get_witnesses(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    reads: Arc<Semaphore>,
) -> Result<Response<String>, GatewayError> {
    use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_READ_KEYS;
    // POST avoids HTTP URI limits for 1,024 wide keys. GET remains compatible with older callers.
    // Each JSON felt needs at most 68 bytes plus a comma; allow space for the root and field names.
    const MAX_BATCH_BODY_BYTES: usize = MAX_COMMITTED_DATA_READ_KEYS * 70 + 256;
    let (root, keys) = if req.method() == Method::POST {
        #[derive(serde::Deserialize)]
        struct WitnessQuery {
            root: Felt,
            keys: Vec<Felt>,
        }
        let body =
            Limited::new(req.into_body(), MAX_BATCH_BODY_BYTES).collect().await.map_err(|_| invalid())?.to_bytes();
        let query: WitnessQuery = serde_json::from_slice(&body).map_err(|_| invalid())?;
        (query.root, query.keys)
    } else {
        let params = get_params_from_request(&req);
        let root =
            params.get("root").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
        let encoded = params.get("keys").ok_or_else(invalid)?;
        // 0x + 64 hex digits + comma per key; reject before allocating the decoded list.
        if encoded.len() > MAX_COMMITTED_DATA_READ_KEYS * 67 {
            return Err(invalid());
        }
        let keys =
            encoded.split(',').map(|key| Felt::from_hex(key).map_err(|_| invalid())).collect::<Result<Vec<_>, _>>()?;
        (root, keys)
    };
    if keys.is_empty() || keys.len() > MAX_COMMITTED_DATA_READ_KEYS {
        return Err(invalid());
    }
    bounded_read(reads, move || {
        Ok(create_json_response(StatusCode::OK, &backend.db.get_committed_data_witnesses(root, &keys)?))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saturating one listener must not consume another listener's configured budget.
    #[tokio::test]
    async fn committed_data_read_budgets_are_independent() {
        let busy = Arc::new(Semaphore::new(1));
        let other = Arc::new(Semaphore::new(2));
        let _held = busy.clone().acquire_owned().await.unwrap();
        assert!(bounded_read(busy, || Ok(Response::new(String::new()))).await.is_err());
        assert!(bounded_read(other.clone(), || Ok(Response::new("ok".into()))).await.is_ok());
        assert_eq!(other.available_permits(), 2);
    }

    /// Dropping an HTTP connection cannot cancel the task awaiting an admitted blocking read.
    /// Shutdown closes keep-alive intake and must wait for that read's final mutation.
    #[rstest::rstest]
    #[case::disconnected(true)]
    #[case::keep_alive(false)]
    #[tokio::test]
    async fn committed_data_gateway_shutdown_drains_reads_after_disconnect(#[case] disconnect: bool) {
        use hyper::{server::conn::http1, service::service_fn};
        use hyper_util::rt::TokioIo;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::{TcpListener, TcpStream},
            task::JoinSet,
        };
        use tokio_util::task::TaskTracker;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = TaskTracker::new();
        let serving_requests = requests.clone();
        let mut connections = JoinSet::new();
        let reads = Arc::new(Semaphore::new(1));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (started, ready) = tokio::sync::oneshot::channel();
        let fixture = Arc::new(std::sync::Mutex::new(Some((started, gate))));
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = completed.clone();
        connections.spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(move |_request| {
                let fixture = fixture.lock().unwrap().take().expect("No request may be admitted after shutdown");
                let reads = reads.clone();
                let completed = worker_completed.clone();
                let request = serving_requests.spawn(async move {
                    bounded_read(reads, move || {
                        fixture.0.send(()).unwrap();
                        let _ = fixture.1.recv_timeout(Duration::from_secs(10));
                        completed.store(true, Ordering::SeqCst);
                        Ok(Response::new("complete".into()))
                    })
                    .await
                    .unwrap()
                    .map(|body| http_body_util::Full::new(bytes::Bytes::from(body)))
                });
                async move { request.await.map_err(std::io::Error::other) }
            });
            let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
        });
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        ready.await.unwrap();
        let mut stream = (!disconnect).then_some(stream);
        let mut stopping = Box::pin(crate::service::stop_gateway(&mut connections, &requests));
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut stopping).await.is_err());
        assert!(!completed.load(Ordering::SeqCst), "Shutdown must wait for the admitted read");
        if let Some(stream) = &mut stream {
            // The old socket cannot submit another keep-alive request while the first DB
            // task is draining. EOF/reset is expected; no second service invocation occurs.
            let _ = stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await;
            let result = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut [0; 1])).await.unwrap();
            assert!(result.is_err() || result.unwrap() == 0);
        }
        drop(release); // Releases even if this test panics before sending.
        tokio::time::timeout(Duration::from_secs(1), stopping).await.unwrap();
        assert!(completed.load(Ordering::SeqCst));
        assert!(requests.is_empty());
    }
}
