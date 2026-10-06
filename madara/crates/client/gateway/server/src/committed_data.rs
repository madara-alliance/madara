//! Read-only dataset replication and witness retrieval. No ingestion route exists here.
use crate::{
    error::GatewayError,
    helpers::{create_json_response, get_params_from_request},
};
use hyper::{body::Incoming, Request, Response, StatusCode};
use mc_db::{MadaraBackend, MadaraStorageRead};
use mp_gateway::{
    committed_data::CommittedDataPage,
    error::{StarknetError, StarknetErrorCode},
};
use starknet_types_core::felt::Felt;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

static READS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(4)));

fn invalid() -> GatewayError {
    StarknetError::new(StarknetErrorCode::MalformedRequest, "Invalid committed-data query".into()).into()
}

/// Validates query bounds before scheduling blocking DB work; the permit survives cancellation.
pub(crate) async fn get(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    kind: &'static str,
) -> Result<Response<String>, GatewayError> {
    let params = get_params_from_request(&req);
    let root = params.get("root").map(|v| Felt::from_hex(v).map_err(|_| invalid())).transpose()?;
    let start = params.get("start").map(|v| v.parse::<u32>().map_err(|_| invalid())).transpose()?.unwrap_or(0);
    let index = params.get("index").map(|v| v.parse::<u32>().map_err(|_| invalid())).transpose()?;
    let after = params.get("after").map(|v| Felt::from_hex(v).map_err(|_| invalid())).transpose()?;
    if kind != "roots" && root.is_none() {
        return Err(invalid());
    }
    if kind == "page" && (start % 4096 != 0 || start >= 1 << 19) {
        return Err(invalid());
    }
    if kind == "witness" && index.is_none_or(|v| v >= 1 << 19) {
        return Err(invalid());
    }
    let permit = READS.clone().try_acquire_owned().map_err(|_| GatewayError::from(StarknetError::rate_limited()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match kind {
            "roots" => Ok(create_json_response(StatusCode::OK, &backend.db.list_committed_data_roots(after)?)),
            "page" => {
                let root = root.ok_or_else(invalid)?;
                let page = backend.db.get_committed_data_page(root, start)?.map(|(count, values)| CommittedDataPage {
                    root,
                    start,
                    count,
                    values,
                });
                Ok(create_json_response(StatusCode::OK, &page))
            }
            "witness" => Ok(create_json_response(
                StatusCode::OK,
                &backend.committed_data_witness(root.ok_or_else(invalid)?, index.ok_or_else(invalid)?)?,
            )),
            _ => Err(invalid()),
        }
    })
    .await
    .map_err(|e| GatewayError::from(anyhow::anyhow!(e)))?
}
