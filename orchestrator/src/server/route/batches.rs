use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::instrument;

use super::super::service::batches::BatchService;
use super::super::types::{
    AggregatorBatchListResponse, ApiResponse, BatchQuery, BlockRouteResult, SnosBatchListResponse,
};
use crate::core::config::Config;

#[instrument(skip(config))]
async fn handle_query_snos_batches(
    Query(query): Query<BatchQuery>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    let response = BatchService::query_snos_batches(config.as_ref(), query).await?;

    Ok(Json(ApiResponse::<SnosBatchListResponse>::success_with_data(
        response,
        Some("Successfully fetched SNOS batches".to_string()),
    ))
    .into_response())
}

#[instrument(skip(config))]
async fn handle_query_aggregator_batches(
    Query(query): Query<BatchQuery>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    let response = BatchService::query_aggregator_batches(config.as_ref(), query).await?;

    Ok(Json(ApiResponse::<AggregatorBatchListResponse>::success_with_data(
        response,
        Some("Successfully fetched aggregator batches".to_string()),
    ))
    .into_response())
}

pub(super) fn batch_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/snos", get(handle_query_snos_batches))
        .route("/aggregator", get(handle_query_aggregator_batches))
        .with_state(config)
}
