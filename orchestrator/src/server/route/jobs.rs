use std::sync::Arc;

use axum::extract::{Json, Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use opentelemetry::KeyValue;
use serde::Deserialize;
use tracing::{error, info, instrument};
use uuid::Uuid;

use super::super::error::JobRouteError;
use super::super::types::{ApiResponse, BlockJobStatusResponse, JobRouteResult, JobStatusResponseItem};

/// Request body for job operations that require a job ID
#[derive(Deserialize)]
struct JobRequest {
    pub id: String,
}
use crate::core::config::Config;
use crate::types::jobs::job_item::JobItem;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::service::JobService;

/// Handles HTTP requests to process a job.
///
/// This endpoint initiates the processing of a job identified by its UUID. It performs the
/// following:
/// 1. Validates and parses the job ID from the request body
/// 2. Calls the job processing logic
/// 3. Records metrics for successful/failed operations
/// 4. Returns an appropriate API response
///
/// # Arguments
/// * `Json(JobRequest { id })` - The job ID from the request body
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult` - Success response or error details
///
/// # Errors
/// * `JobRouteError::InvalidId` - If the provided ID is not a valid UUID
/// * `JobRouteError::ProcessingError` - If job processing fails
#[cfg_attr(debug_assertions, axum::debug_handler)]
#[instrument(skip(config), fields(job_id = %id))]
async fn handle_process_job_request(
    State(config): State<Arc<Config>>,
    Json(JobRequest { id }): Json<JobRequest>,
) -> JobRouteResult {
    let job_id = Uuid::parse_str(&id).map_err(|_| JobRouteError::InvalidId(id.clone()))?;

    match JobService::queue_job_for_processing(job_id, config.clone()).await {
        Ok(_) => {
            info!("Job queued for processing successfully");
            ORCHESTRATOR_METRICS
                .successful_job_operations
                .add(1.0, &[KeyValue::new("operation_type", "queue_process")]);
            Ok(Json(ApiResponse::<()>::success(Some(format!("Job with id {} queued for processing", id))))
                .into_response())
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
/// * `Json(JobRequest { id })` - The job ID from the request body
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult` - Success response or error details
///
/// # Errors
/// * `JobRouteError::InvalidId` - If the provided ID is not a valid UUID
/// * `JobRouteError::ProcessingError` - If queueing for verification fails
#[cfg_attr(debug_assertions, axum::debug_handler)]
#[instrument(skip(config), fields(job_id = %id))]
async fn handle_verify_job_request(
    State(config): State<Arc<Config>>,
    Json(JobRequest { id }): Json<JobRequest>,
) -> JobRouteResult {
    let job_id = Uuid::parse_str(&id).map_err(|_| JobRouteError::InvalidId(id.clone()))?;

    match JobService::queue_job_for_verification(job_id, config.clone()).await {
        Ok(_) => {
            info!("Job queued for verification successfully");
            ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &[KeyValue::new("operation_type", "queue_verify")]);
            Ok(Json(ApiResponse::<()>::success(Some(format!("Job with id {} queued for verification", id))))
                .into_response())
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
/// * `Json(JobRequest { id })` - The job ID from the request body
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
    State(config): State<Arc<Config>>,
    Json(JobRequest { id }): Json<JobRequest>,
) -> JobRouteResult {
    let job_id = Uuid::parse_str(&id).map_err(|_| JobRouteError::InvalidId(id.clone()))?;

    match JobHandlerService::retry_job(job_id, config.clone()).await {
        Ok(_) => {
            info!("Job retry initiated successfully");
            ORCHESTRATOR_METRICS.successful_job_operations.add(
                1.0,
                &[KeyValue::new("operation_type", "process_job"), KeyValue::new("operation_info", "retry_job")],
            );

            Ok(Json(ApiResponse::<()>::success(Some(format!("Job with id {} retry initiated", id)))).into_response())
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
    Router::new()
        .nest("/", job_trigger_router(config.clone()))
        .route("/block/:block_number/status", get(handle_get_job_status_by_block_request).with_state(config))
}

/// Handles HTTP requests to get job statuses by block number.
///
/// This endpoint retrieves all job statuses for a given block number.
///
/// # Arguments
/// * `Path(block_number)` - The block number extracted from the URL path
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult<BlockJobStatusResponse>` - Success response with job statuses or error details
#[instrument(skip(config), fields(block_number = %block_number))]
async fn handle_get_job_status_by_block_request(
    Path(block_number): Path<u64>,
    State(config): State<Arc<Config>>,
) -> JobRouteResult {
    match config.database().get_jobs_by_block_number(block_number).await {
        Ok(jobs) => {
            let mut job_status_items = Vec::new();
            for job in jobs {
                // ProofRegistration is now always included if found
                job_status_items.push(JobStatusResponseItem { job_type: job.job_type, id: job.id, status: job.status });
            }
            info!(count = job_status_items.len(), "Successfully fetched job statuses for block");
            Ok(Json(ApiResponse::<BlockJobStatusResponse>::success_with_data(
                BlockJobStatusResponse { jobs: job_status_items },
                Some(format!("Successfully fetched job statuses for block {}", block_number)),
            ))
            .into_response())
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch job statuses for block");
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Get all failed job, support to return all the Failed job in the application
///
///
/// # Arguments
/// * `Json(JobRequest { id })` - The job ID from the request body
/// * `State(config)` - Shared application configuration
///
/// # Returns
/// * `JobRouteResult` - Success response or error details
///
/// # Errors
/// * `JobRouteError::InvalidId` - If the provided ID is not a valid UUID
/// * `JobRouteError::ProcessingError` - If retry attempt fails
#[instrument(skip(config))]
async fn get_failed_job(State(config): State<Arc<Config>>) -> JobRouteResult {
    match JobHandlerService::get_failed_jobs(config.clone()).await {
        Ok(jobs) => {
            info!("Successfully fetched failed jobs");
            Ok(Json(ApiResponse::<Vec<JobItem>>::success_with_data(
                jobs,
                Some("Successfully fetched failed jobs".to_string()),
            ))
            .into_response())
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch failed jobs");
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Creates the nested router for job trigger endpoints.
///
/// Sets up specific routes for processing, verifying, and retrying jobs.
///
/// # Arguments
/// * `config` - Shared application configuration
///
/// # Returns
/// * `Router` - Configured router with trigger endpoints
pub(super) fn job_trigger_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/process", post(handle_process_job_request))
        .route("/verify", post(handle_verify_job_request))
        .route("/retry", post(handle_retry_job_request))
        .route("/status/failed", get(get_failed_job))
        .with_state(config)
}
