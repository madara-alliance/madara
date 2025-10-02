use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::{error, instrument};

use super::super::types::{ApiResponse, BlockRouteResult, BlockStatusResponse};
use crate::core::config::Config;
use crate::server::error::BlockRouteError;

#[instrument(skip(config), fields(block_number = %block_number))]
async fn handle_block_to_batch(
    Path(block_number): Path<u64>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    match config.database().get_batch_for_block(block_number).await {
        Ok(Some(batch)) => {
            Ok(Json(ApiResponse::<BlockStatusResponse>::success_with_data(
                BlockStatusResponse { batch_number: batch.index },
                Some(format!("Successfully fetched batch for block {}", block_number)),
            )).into_response())
        }
        Ok(None) => {
            tracing::warn!("No batch found for block {}, skipping for now", block_number);
            Err(BlockRouteError::NotFound(format!("No batch found for block {}", block_number)))
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch batch for block");
            Err(BlockRouteError::DatabaseError(e.to_string()))
        }
    }
}

pub(super) fn block_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/batch-for-block/:block_number", get(handle_block_to_batch))
        .with_state(config)
}
