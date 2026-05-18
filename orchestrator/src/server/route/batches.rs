use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};

use super::super::error::BlockRouteError;
use super::super::types::{
    AggregatorBatchDetailsResponse, ApiResponse, BatchDetailsResponse, BlockRouteResult, SnosBatchDetailsResponse,
    SnosBatchMetricsResponse,
};
use crate::core::config::Config;
use crate::types::batch::{AggregatorBatch, SnosBatch};

async fn handle_snos_batch_details(
    Path(snos_batch_index): Path<u64>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    let snos_batch = config
        .database()
        .get_snos_batches_by_indices(vec![snos_batch_index])
        .await
        .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| BlockRouteError::NotFound(format!("No SNOS batch found for index {}", snos_batch_index)))?;

    let aggregator_batch = match snos_batch.aggregator_batch_index {
        Some(aggregator_batch_index) => config
            .database()
            .get_aggregator_batches_by_indexes(vec![aggregator_batch_index])
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
            .into_iter()
            .next(),
        None => None,
    };

    Ok(Json(ApiResponse::<BatchDetailsResponse>::success_with_data(
        BatchDetailsResponse {
            snos_batch: snapshot_snos_batch_details(&snos_batch),
            aggregator_batch: aggregator_batch.as_ref().map(snapshot_aggregator_batch_details),
        },
        Some(format!("Successfully fetched details for SNOS batch {}", snos_batch_index)),
    ))
    .into_response())
}

fn snapshot_snos_batch_details(batch: &SnosBatch) -> SnosBatchDetailsResponse {
    SnosBatchDetailsResponse {
        index: batch.index,
        aggregator_batch_index: batch.aggregator_batch_index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        status: batch.status.clone(),
        created_at: batch.created_at,
        updated_at: batch.updated_at,
        metrics: SnosBatchMetricsResponse {
            state_diff_size: batch.builtin_weights.state_diff_size,
            sierra_gas: batch.builtin_weights.sierra_gas.0,
            proving_gas: batch.builtin_weights.proving_gas.0,
        },
    }
}

fn snapshot_aggregator_batch_details(batch: &AggregatorBatch) -> AggregatorBatchDetailsResponse {
    AggregatorBatchDetailsResponse {
        index: batch.index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        status: batch.status.clone(),
        created_at: batch.created_at,
        updated_at: batch.updated_at,
        blob_len: batch.blob_len,
    }
}

pub(super) fn batch_router(config: Arc<Config>) -> Router {
    Router::new().route("/snos/:snos_batch_index/details", get(handle_snos_batch_details)).with_state(config)
}
