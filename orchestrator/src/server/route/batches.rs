use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::instrument;

use super::super::error::BlockRouteError;
use super::super::types::{
    AggregatorBatchDetailsResponse, AggregatorBatchListResponse, AggregatorBatchQuery, ApiResponse, BatchSortOrder,
    BlockRouteResult, SettlementAggregatorBatchResponse, SettlementSnosBatchResponse, SnosBatchDetailsResponse,
    SnosBatchListResponse, SnosBatchMetricsResponse, SnosBatchQuery,
};
use crate::core::config::Config;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch, SnosBatchStatus};

enum SnosBatchStatusFilter {
    Closed,
    Exact(SnosBatchStatus),
}

enum AggregatorBatchStatusFilter {
    Closed,
    Exact(AggregatorBatchStatus),
}

#[instrument(skip(config), fields(index = ?query.index, status = ?query.status, limit = ?query.limit, sort = ?query.sort))]
async fn handle_query_snos_batches(
    Query(query): Query<SnosBatchQuery>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    let limit = validate_limit(query.limit)?;
    let descending = matches!(query.sort, BatchSortOrder::Desc);
    let status_filter = parse_snos_status(query.status.as_deref())?;

    let batches = if let Some(index) = query.index {
        let mut batches = config
            .database()
            .get_snos_batches_by_indices(vec![index])
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

        if let Some(filter) = status_filter {
            batches.retain(|batch| matches_snos_status(batch, &filter));
        }

        batches
    } else if let Some(filter) = status_filter {
        match filter {
            SnosBatchStatusFilter::Exact(status) => config
                .database()
                .get_snos_batches_by_status(status, limit, None, descending)
                .await
                .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
            SnosBatchStatusFilter::Closed => {
                let mut batches = config
                    .database()
                    .get_snos_batches(None, descending)
                    .await
                    .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;
                batches.retain(|batch| batch.status.is_closed());
                apply_limit(&mut batches, limit);
                batches
            }
        }
    } else {
        config
            .database()
            .get_snos_batches(limit, descending)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
    };

    Ok(Json(ApiResponse::<SnosBatchListResponse>::success_with_data(
        SnosBatchListResponse { batches: batches.iter().map(snapshot_snos_batch_details).collect() },
        Some("Successfully fetched SNOS batches".to_string()),
    ))
    .into_response())
}

#[instrument(skip(config), fields(index = ?query.index, status = ?query.status, limit = ?query.limit, sort = ?query.sort))]
async fn handle_query_aggregator_batches(
    Query(query): Query<AggregatorBatchQuery>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    let limit = validate_limit(query.limit)?;
    let descending = matches!(query.sort, BatchSortOrder::Desc);
    let status_filter = parse_aggregator_status(query.status.as_deref())?;

    let batches = if let Some(index) = query.index {
        let mut batches = config
            .database()
            .get_aggregator_batches_by_indexes(vec![index])
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

        if let Some(filter) = status_filter {
            batches.retain(|batch| matches_aggregator_status(batch, &filter));
        }

        batches
    } else if let Some(filter) = status_filter {
        match filter {
            AggregatorBatchStatusFilter::Exact(status) => config
                .database()
                .get_aggregator_batches_by_status(status, limit, None, descending)
                .await
                .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
            AggregatorBatchStatusFilter::Closed => {
                let mut batches = config
                    .database()
                    .get_aggregator_batches(None, descending)
                    .await
                    .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;
                batches.retain(|batch| batch.status.is_closed());
                apply_limit(&mut batches, limit);
                batches
            }
        }
    } else {
        config
            .database()
            .get_aggregator_batches(limit, descending)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
    };

    Ok(Json(ApiResponse::<AggregatorBatchListResponse>::success_with_data(
        AggregatorBatchListResponse { batches: batches.iter().map(snapshot_aggregator_batch_details).collect() },
        Some("Successfully fetched aggregator batches".to_string()),
    ))
    .into_response())
}

fn validate_limit(limit: Option<i64>) -> Result<Option<i64>, BlockRouteError> {
    match limit {
        Some(value) if value <= 0 => Err(BlockRouteError::InvalidQuery("limit must be a positive integer".to_string())),
        other => Ok(other),
    }
}

fn apply_limit<T>(items: &mut Vec<T>, limit: Option<i64>) {
    if let Some(limit) = limit {
        items.truncate(limit as usize);
    }
}

fn parse_snos_status(status: Option<&str>) -> Result<Option<SnosBatchStatusFilter>, BlockRouteError> {
    let Some(status) = status else {
        return Ok(None);
    };

    let parsed = match status.trim().to_ascii_lowercase().as_str() {
        "open" => SnosBatchStatusFilter::Exact(SnosBatchStatus::Open),
        "closed" => SnosBatchStatusFilter::Closed,
        "snosjobcreated" | "snos_job_created" | "snos-job-created" => {
            SnosBatchStatusFilter::Exact(SnosBatchStatus::SnosJobCreated)
        }
        "completed" => SnosBatchStatusFilter::Exact(SnosBatchStatus::Completed),
        _ => return Err(BlockRouteError::InvalidQuery(format!("unsupported snos batch status '{}'", status))),
    };

    Ok(Some(parsed))
}

fn matches_snos_status(batch: &SnosBatch, filter: &SnosBatchStatusFilter) -> bool {
    match filter {
        SnosBatchStatusFilter::Closed => batch.status.is_closed(),
        SnosBatchStatusFilter::Exact(status) => &batch.status == status,
    }
}

fn parse_aggregator_status(status: Option<&str>) -> Result<Option<AggregatorBatchStatusFilter>, BlockRouteError> {
    let Some(status) = status else {
        return Ok(None);
    };

    let parsed = match status.trim().to_ascii_lowercase().as_str() {
        "open" => AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::Open),
        "closed" => AggregatorBatchStatusFilter::Closed,
        "pendingaggregatorrun" | "pending_aggregator_run" | "pending-aggregator-run" => {
            AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::PendingAggregatorRun)
        }
        "pendingaggregatorverification" | "pending_aggregator_verification" | "pending-aggregator-verification" => {
            AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::PendingAggregatorVerification)
        }
        "readyforstateupdate" | "ready_for_state_update" | "ready-for-state-update" => {
            AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::ReadyForStateUpdate)
        }
        "completed" => AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::Completed),
        "aggregationfailed" | "aggregation_failed" | "aggregation-failed" => {
            AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::AggregationFailed)
        }
        "verificationfailed" | "verification_failed" | "verification-failed" => {
            AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::VerificationFailed)
        }
        "stateupdatefailed" | "state_update_failed" | "state-update-failed" => {
            AggregatorBatchStatusFilter::Exact(AggregatorBatchStatus::StateUpdateFailed)
        }
        _ => return Err(BlockRouteError::InvalidQuery(format!("unsupported aggregator batch status '{}'", status))),
    };

    Ok(Some(parsed))
}

fn matches_aggregator_status(batch: &AggregatorBatch, filter: &AggregatorBatchStatusFilter) -> bool {
    match filter {
        AggregatorBatchStatusFilter::Closed => batch.status.is_closed(),
        AggregatorBatchStatusFilter::Exact(status) => &batch.status == status,
    }
}

pub(super) fn snapshot_snos_batch(batch: &SnosBatch) -> SettlementSnosBatchResponse {
    SettlementSnosBatchResponse {
        index: batch.index,
        aggregator_batch_index: batch.aggregator_batch_index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        status: batch.status.clone(),
        created_at: batch.created_at,
        updated_at: batch.updated_at,
    }
}

pub(super) fn snapshot_aggregator_batch(batch: &AggregatorBatch) -> SettlementAggregatorBatchResponse {
    SettlementAggregatorBatchResponse {
        index: batch.index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        status: batch.status.clone(),
        created_at: batch.created_at,
        updated_at: batch.updated_at,
    }
}

fn snapshot_snos_batch_details(batch: &SnosBatch) -> SnosBatchDetailsResponse {
    SnosBatchDetailsResponse {
        batch: snapshot_snos_batch(batch),
        metrics: SnosBatchMetricsResponse {
            state_diff_size: batch.builtin_weights.state_diff_size,
            sierra_gas: batch.builtin_weights.sierra_gas.0,
            proving_gas: batch.builtin_weights.proving_gas.0,
        },
    }
}

fn snapshot_aggregator_batch_details(batch: &AggregatorBatch) -> AggregatorBatchDetailsResponse {
    AggregatorBatchDetailsResponse { batch: snapshot_aggregator_batch(batch), blob_len: batch.blob_len }
}

pub(super) fn batch_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/snos", get(handle_query_snos_batches))
        .route("/aggregator", get(handle_query_aggregator_batches))
        .with_state(config)
}
