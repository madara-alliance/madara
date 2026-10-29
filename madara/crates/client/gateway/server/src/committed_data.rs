//! Read-only dataset replication and witness retrieval. No ingestion route exists here.
use crate::{
    error::GatewayError,
    helpers::{create_json_response, get_params_from_request},
};
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use hyper::{body::Incoming, Request, Response, StatusCode};
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
    let params = get_params_from_request(&req);
    let root = params.get("root").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
    let encoded = params.get("keys").ok_or_else(invalid)?;
    // 0x + 64 hex digits + comma per key; reject before allocating the decoded list.
    if encoded.len() > MAX_COMMITTED_DATA_READ_KEYS * 67 {
        return Err(invalid());
    }
    let keys =
        encoded.split(',').map(|key| Felt::from_hex(key).map_err(|_| invalid())).collect::<Result<Vec<_>, _>>()?;
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
}
