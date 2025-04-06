use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use opentelemetry::KeyValue;
use tracing::{error, info, instrument};
use uuid::Uuid;

use super::super::error::JobRouteError;
use super::super::types::{ApiResponse, JobId, JobRouteResult};
use crate::worker::service::JobService;
use crate::{core::config::Config, utils::metrics::ORCHESTRATOR_METRICS};

/// Handles HTTP requests to process a job.
///
/// This endpoint initiates the processing of a job identified by its UUID. It performs the
/// following:
/// 1. Validates and parses the job ID from the URL path parameter
/// 2. Calls the job processing logic
/// 3. Records metrics for successful/failed operations
/// 4. Returns an appropriate API response
///
/// # Arguments
/// * `Path(JobId { id })` - The job ID extracted from the URL path
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult` - Success response or error details
///
/// # Errors
/// * `JobRouteError::InvalidId` - If the provided ID is not a valid UUID
/// * `JobRouteError::ProcessingError` - If job processing fails
#[instrument(skip(config), fields(job_id = %id))]
async fn handle_process_job_request(
    Path(JobId { id }): Path<JobId>,
    State(config): State<Arc<Config>>,
) -> JobRouteResult {
    let job_id = Uuid::parse_str(&id).map_err(|_| JobRouteError::InvalidId(id.clone()))?;

    match JobService::queue_job_for_processing(job_id, config.clone()).await {
        Ok(_) => {
            info!("Job queued for processing successfully");
            ORCHESTRATOR_METRICS
                .successful_job_operations
                .add(1.0, &[KeyValue::new("operation_type", "queue_process")]);
            Ok(Json(ApiResponse::success(Some(format!("Job with id {} queued for processing", id)))).into_response())
        }
        Err(e) => {
            error!(error = %e, "Failed to queue job for processing");
            ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &[KeyValue::new("operation_type", "queue_process")]);
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Handles HTTP requests to verify a job's status.
///
/// This endpoint queues the job for verification by:
/// 1. Validates and parses the job ID
/// 2. Adds the job to the verification queue
/// 3. Resets verification attempt counter
/// 4. Records metrics for the queue operation
/// 5. Returns immediate response
///
/// # Arguments
/// * `Path(JobId { id })` - The job ID extracted from the URL path
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult` - Success response or error details
///
/// # Errors
/// * `JobRouteError::InvalidId` - If the provided ID is not a valid UUID
/// * `JobRouteError::ProcessingError` - If queueing for verification fails
#[instrument(skip(config), fields(job_id = %id))]
async fn handle_verify_job_request(
    Path(JobId { id }): Path<JobId>,
    State(config): State<Arc<Config>>,
) -> JobRouteResult {
    let job_id = Uuid::parse_str(&id).map_err(|_| JobRouteError::InvalidId(id.clone()))?;

    match JobService::queue_job_for_verification(job_id, config.clone()).await {
        Ok(_) => {
            info!("Job queued for verification successfully");
            ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &[KeyValue::new("operation_type", "queue_verify")]);
            Ok(Json(ApiResponse::success(Some(format!("Job with id {} queued for verification", id)))).into_response())
        }
        Err(e) => {
            error!(error = %e, "Failed to queue job for verification");
            ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &[KeyValue::new("operation_type", "queue_verify")]);
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Handles HTTP requests to retry a failed job.
///
/// This endpoint attempts to retry a previously failed job. It:
/// 1. Validates and parses the job ID
/// 2. Initiates the retry process
/// 3. Records metrics with additional retry context
/// 4. Returns the retry attempt result
///
/// # Arguments
/// * `Path(JobId { id })` - The job ID extracted from the URL path
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult` - Success response or error details
///
/// # Errors
/// * `JobRouteError::InvalidId` - If the provided ID is not a valid UUID
/// * `JobRouteError::ProcessingError` - If retry attempt fails
#[instrument(skip(config), fields(job_id = %id))]
async fn handle_retry_job_request(
    Path(JobId { id }): Path<JobId>,
    State(config): State<Arc<Config>>,
) -> JobRouteResult {
    let job_id = Uuid::parse_str(&id).map_err(|_| JobRouteError::InvalidId(id.clone()))?;

    match JobService::retry_job(job_id, config.clone()).await {
        Ok(_) => {
            info!("Job retry initiated successfully");
            ORCHESTRATOR_METRICS.successful_job_operations.add(
                1.0,
                &[KeyValue::new("operation_type", "process_job"), KeyValue::new("operation_info", "retry_job")],
            );

            Ok(Json(ApiResponse::success(Some(format!("Job with id {} retry initiated", id)))).into_response())
        }
        Err(e) => {
            error!(error = %e, "Failed to retry job");
            ORCHESTRATOR_METRICS.failed_job_operations.add(
                1.0,
                &[KeyValue::new("operation_type", "process_job"), KeyValue::new("operation_info", "retry_job")],
            );
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Creates a router for job-related endpoints.
///
/// This function sets up the main router for all job-related operations,
/// nesting the specific job trigger endpoints under the "/jobs" path.
///
/// # Arguments
/// * `config` - Shared application configuration
///
/// # Returns
/// * `Router` - Configured router with all job endpoints
pub fn job_router(config: Arc<Config>) -> Router {
    Router::new().nest("/:id", job_trigger_router(config.clone()))
}

/// Creates the nested router for job trigger endpoints.
///
/// Sets up specific routes for processing, verifying, and retrying jobs.
/// All endpoints are configured as GET requests and share the application config.
///
/// # Arguments
/// * `config` - Shared application configuration
///
/// # Returns
/// * `Router` - Configured router with trigger endpoints
pub(super) fn job_trigger_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/process", get(handle_process_job_request))
        .route("/verify", get(handle_verify_job_request))
        .route("/retry", get(handle_retry_job_request))
        .with_state(config)
}
