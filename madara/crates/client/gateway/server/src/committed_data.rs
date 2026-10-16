//! Read-only dataset replication and witness retrieval. No ingestion route exists here.
use crate::{
    error::GatewayError,
    helpers::{create_json_response, get_params_from_request},
};
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use hyper::{body::Incoming, Request, Response, StatusCode};
use mc_db::{MadaraBackend, MadaraStorageRead};
use mp_gateway::{
    committed_data::{CommittedDataPage, VALUES_PER_PAGE},
    error::{StarknetError, StarknetErrorCode},
};
use starknet_types_core::felt::Felt;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

const MAX_CONCURRENT_READS: usize = 4;
static READS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_READS)));

fn invalid() -> GatewayError {
    StarknetError::new(StarknetErrorCode::MalformedRequest, "Invalid committed-data query".into()).into()
}

/// Keep the permit in the blocking task even if the HTTP client disconnects.
async fn bounded_read(
    read: impl FnOnce() -> Result<Response<String>, GatewayError> + Send + 'static,
) -> Result<Response<String>, GatewayError> {
    let permit = READS.clone().try_acquire_owned().map_err(|_| GatewayError::from(StarknetError::rate_limited()))?;
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
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let after = params.get("after").map(|value| Felt::from_hex(value).map_err(|_| invalid())).transpose()?;
    bounded_read(move || Ok(create_json_response(StatusCode::OK, &backend.db.list_committed_data_roots(after)?))).await
}

/// Return one bounded value slice; consumers authenticate the assembled root.
pub(crate) async fn get_page(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let root = params.get("root").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
    let start = params.get("start").map(|value| value.parse::<u32>().map_err(|_| invalid())).transpose()?.unwrap_or(0);
    if (start as usize) % VALUES_PER_PAGE != 0 || (start as usize) >= MAX_COMMITTED_DATA_VALUES {
        return Err(invalid());
    }
    bounded_read(move || {
        let page = backend.db.get_committed_data_page(root, start)?.map(|(count, values)| CommittedDataPage {
            root,
            start,
            count,
            values,
        });
        Ok(create_json_response(StatusCode::OK, &page))
    })
    .await
}

/// Return an indexed Merkle witness used by the proving pipeline.
pub(crate) async fn get_witness(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let root = params.get("root").ok_or_else(invalid).and_then(|value| Felt::from_hex(value).map_err(|_| invalid()))?;
    let index =
        params.get("index").ok_or_else(invalid).and_then(|value| value.parse::<u32>().map_err(|_| invalid()))?;
    if (index as usize) >= MAX_COMMITTED_DATA_VALUES {
        return Err(invalid());
    }
    bounded_read(move || Ok(create_json_response(StatusCode::OK, &backend.committed_data_witness(root, index)?))).await
}
