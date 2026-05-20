use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::instrument;

use super::super::error::BlockRouteError;
use super::super::types::{
    AggregatorBatchListResponse, ApiResponse, BatchQuery, BatchSortOrder, BlockRouteResult, SnosBatchListResponse,
};
use crate::core::config::Config;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch, SnosBatchStatus};

enum BatchStatusFilter<T> {
    Closed,
    Exact(T),
}

#[instrument(skip(config))]
async fn handle_query_snos_batches(
    Query(query): Query<BatchQuery>,
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
            BatchStatusFilter::Exact(status) => config
                .database()
                .get_snos_batches_by_status(status, limit, None, descending)
                .await
                .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
            BatchStatusFilter::Closed => get_closed_snos_batches(config.as_ref(), limit, descending).await?,
        }
    } else {
        config
            .database()
            .get_snos_batches(limit, descending)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
    };

    Ok(Json(ApiResponse::<SnosBatchListResponse>::success_with_data(
        SnosBatchListResponse { batches: batches.iter().map(Into::into).collect() },
        Some("Successfully fetched SNOS batches".to_string()),
    ))
    .into_response())
}

#[instrument(skip(config))]
async fn handle_query_aggregator_batches(
    Query(query): Query<BatchQuery>,
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
            BatchStatusFilter::Exact(status) => config
                .database()
                .get_aggregator_batches_by_status(status, limit, None, descending)
                .await
                .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
            BatchStatusFilter::Closed => get_closed_aggregator_batches(config.as_ref(), limit, descending).await?,
        }
    } else {
        config
            .database()
            .get_aggregator_batches(limit, descending)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
    };

    Ok(Json(ApiResponse::<AggregatorBatchListResponse>::success_with_data(
        AggregatorBatchListResponse { batches: batches.iter().map(Into::into).collect() },
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

async fn get_closed_snos_batches(
    config: &Config,
    limit: Option<i64>,
    descending: bool,
) -> Result<Vec<SnosBatch>, BlockRouteError> {
    let mut batches = Vec::new();

    for status in [SnosBatchStatus::SnosJobCreated, SnosBatchStatus::Completed] {
        batches.extend(
            config
                .database()
                .get_snos_batches_by_status(status, limit, None, descending)
                .await
                .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
        );
    }

    batches.sort_by_key(|batch| batch.index);
    if descending {
        batches.reverse();
    }
    apply_limit(&mut batches, limit);
    Ok(batches)
}

async fn get_closed_aggregator_batches(
    config: &Config,
    limit: Option<i64>,
    descending: bool,
) -> Result<Vec<AggregatorBatch>, BlockRouteError> {
    let mut batches = Vec::new();

    for status in [
        AggregatorBatchStatus::Closed,
        AggregatorBatchStatus::PendingAggregatorRun,
        AggregatorBatchStatus::PendingAggregatorVerification,
        AggregatorBatchStatus::ReadyForStateUpdate,
        AggregatorBatchStatus::Completed,
        AggregatorBatchStatus::AggregationFailed,
        AggregatorBatchStatus::VerificationFailed,
        AggregatorBatchStatus::StateUpdateFailed,
    ] {
        batches.extend(
            config
                .database()
                .get_aggregator_batches_by_status(status, limit, None, descending)
                .await
                .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
        );
    }

    batches.sort_by_key(|batch| batch.index);
    if descending {
        batches.reverse();
    }
    apply_limit(&mut batches, limit);
    Ok(batches)
}

fn parse_snos_status(status: Option<&str>) -> Result<Option<BatchStatusFilter<SnosBatchStatus>>, BlockRouteError> {
    parse_status("snos", status)
}

fn matches_snos_status(batch: &SnosBatch, filter: &BatchStatusFilter<SnosBatchStatus>) -> bool {
    matches_status(&batch.status, filter, SnosBatchStatus::is_closed)
}

fn parse_aggregator_status(
    status: Option<&str>,
) -> Result<Option<BatchStatusFilter<AggregatorBatchStatus>>, BlockRouteError> {
    parse_status("aggregator", status)
}

fn matches_aggregator_status(batch: &AggregatorBatch, filter: &BatchStatusFilter<AggregatorBatchStatus>) -> bool {
    matches_status(&batch.status, filter, AggregatorBatchStatus::is_closed)
}

fn parse_status<T>(kind: &str, status: Option<&str>) -> Result<Option<BatchStatusFilter<T>>, BlockRouteError>
where
    T: FromStr,
{
    let Some(status) = status else {
        return Ok(None);
    };
    let status = status.trim();

    if status.eq_ignore_ascii_case("closed") {
        return Ok(Some(BatchStatusFilter::Closed));
    }

    status
        .to_ascii_lowercase()
        .parse()
        .map(BatchStatusFilter::Exact)
        .map(Some)
        .map_err(|_| BlockRouteError::InvalidQuery(format!("unsupported {kind} batch status '{status}'")))
}

fn matches_status<T>(status: &T, filter: &BatchStatusFilter<T>, is_closed: impl Fn(&T) -> bool) -> bool
where
    T: PartialEq,
{
    match filter {
        BatchStatusFilter::Closed => is_closed(status),
        BatchStatusFilter::Exact(expected) => status == expected,
    }
}

pub(super) fn batch_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/snos", get(handle_query_snos_batches))
        .route("/aggregator", get(handle_query_aggregator_batches))
        .with_state(config)
}
