use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use opentelemetry::KeyValue;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;

use super::super::error::JobRouteError;
use super::super::service::admin::AdminService;
use super::super::types::{ApiResponse, JobRouteResult};
use crate::core::config::Config;
use crate::types::jobs::types::JobType;
use crate::utils::metrics::ORCHESTRATOR_METRICS;

/// Query parameters for filtering jobs by types
#[derive(Debug, Deserialize)]
pub struct JobTypeFilter {
    /// Optional job types filter (comma-separated or multiple query params)
    /// If empty, all job types will be processed
    #[serde(default)]
    pub job_type: Vec<JobType>,
}

/// Response for bulk job operations
#[derive(Debug, Serialize)]
pub struct BulkJobResponse {
    /// Number of jobs successfully queued
    pub success_count: u64,
    /// List of job IDs that were successfully queued
    pub successful_job_ids: Vec<Uuid>,
    /// Number of jobs that failed to queue
    pub failed_count: u64,
    /// List of job IDs that failed to queue
    pub failed_job_ids: Vec<Uuid>,
}

/// Handles HTTP requests to retry all failed jobs.
///
/// This endpoint initiates the retry process for all jobs in Failed status.
/// Optionally filters by job types if provided in query parameters.
///
/// # Arguments
/// * `State(config)` - Shared application configuration
/// * `Query(filter)` - Optional job types filter from query parameters
///
/// # Returns
/// * `JobRouteResult` - Success response with job count and IDs, or error details
///
/// # Errors
/// * `JobRouteError::ProcessingError` - If the bulk retry operation fails
async fn handle_retry_all_failed_jobs(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    // Check if admin is enabled
    if !config.server_config().admin_enabled {
        error!("Admin endpoints are disabled");
        return Err(JobRouteError::ProcessingError(
            "Admin endpoints are disabled. Enable with --admin-enabled flag.".to_string(),
        ));
    }

    info!(job_types = ?filter.job_type, "Admin: Retry all failed jobs request received");

    match AdminService::retry_all_failed_jobs(filter.job_type.clone(), config.clone()).await {
        Ok(result) => {
            info!(
                success = result.success_count,
                failed = result.failed_count,
                job_types = ?filter.job_type,
                "Admin: Completed retry of failed jobs"
            );

            ORCHESTRATOR_METRICS.successful_job_operations.add(
                result.success_count as f64,
                &[KeyValue::new("operation_type", "admin_retry_failed"), KeyValue::new("admin", "true")],
            );

            if result.failed_count > 0 {
                ORCHESTRATOR_METRICS.failed_job_operations.add(
                    result.failed_count as f64,
                    &[KeyValue::new("operation_type", "admin_retry_failed"), KeyValue::new("admin", "true")],
                );
            }

            let message = if result.success_count == 0 && result.failed_count == 0 {
                "No failed jobs to retry".to_string()
            } else if result.failed_count > 0 {
                format!(
                    "Queued {} job(s) for retry. {} job(s) failed to queue.",
                    result.success_count, result.failed_count
                )
            } else {
                format!("Successfully queued {} job(s) for retry", result.success_count)
            };

            Ok(Json(ApiResponse::<BulkJobResponse>::success_with_data(
                BulkJobResponse {
                    success_count: result.success_count,
                    successful_job_ids: result.successful_job_ids,
                    failed_count: result.failed_count,
                    failed_job_ids: result.failed_job_ids,
                },
                Some(message),
            ))
            .into_response())
        }
        Err(e) => {
            error!(error = %e, job_types = ?filter.job_type, "Admin: Failed to retry failed jobs");
            ORCHESTRATOR_METRICS
                .failed_job_operations
                .add(1.0, &[KeyValue::new("operation_type", "admin_retry_failed"), KeyValue::new("admin", "true")]);
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Handles HTTP requests to re-queue all pending verification jobs.
///
/// This endpoint re-adds all jobs in PendingVerification status to their
/// respective verification queues based on their job types.
///
/// # Arguments
/// * `State(config)` - Shared application configuration
/// * `Query(filter)` - Optional job types filter from query parameters
///
/// # Returns
/// * `JobRouteResult` - Success response with job count and IDs, or error details
///
/// # Errors
/// * `JobRouteError::ProcessingError` - If the bulk requeue operation fails
async fn handle_requeue_pending_verification(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    // Check if admin is enabled
    if !config.server_config().admin_enabled {
        error!("Admin endpoints are disabled");
        return Err(JobRouteError::ProcessingError(
            "Admin endpoints are disabled. Enable with --admin-enabled flag.".to_string(),
        ));
    }

    info!(job_types = ?filter.job_type, "Admin: Requeue pending verification jobs request received");

    match AdminService::requeue_pending_verification(filter.job_type.clone(), config.clone()).await {
        Ok(result) => {
            info!(
                success = result.success_count,
                failed = result.failed_count,
                job_types = ?filter.job_type,
                "Admin: Completed requeue of pending verification jobs"
            );

            ORCHESTRATOR_METRICS.successful_job_operations.add(
                result.success_count as f64,
                &[KeyValue::new("operation_type", "admin_requeue_verification"), KeyValue::new("admin", "true")],
            );

            if result.failed_count > 0 {
                ORCHESTRATOR_METRICS.failed_job_operations.add(
                    result.failed_count as f64,
                    &[KeyValue::new("operation_type", "admin_requeue_verification"), KeyValue::new("admin", "true")],
                );
            }

            let message = if result.success_count == 0 && result.failed_count == 0 {
                "No pending verification jobs to requeue".to_string()
            } else if result.failed_count > 0 {
                format!(
                    "Re-queued {} job(s) for verification. {} job(s) failed to queue.",
                    result.success_count, result.failed_count
                )
            } else {
                format!("Successfully re-queued {} job(s) for verification", result.success_count)
            };

            Ok(Json(ApiResponse::<BulkJobResponse>::success_with_data(
                BulkJobResponse {
                    success_count: result.success_count,
                    successful_job_ids: result.successful_job_ids,
                    failed_count: result.failed_count,
                    failed_job_ids: result.failed_job_ids,
                },
                Some(message),
            ))
            .into_response())
        }
        Err(e) => {
            error!(error = %e, job_types = ?filter.job_type, "Admin: Failed to requeue pending verification jobs");
            ORCHESTRATOR_METRICS.failed_job_operations.add(
                1.0,
                &[KeyValue::new("operation_type", "admin_requeue_verification"), KeyValue::new("admin", "true")],
            );
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Handles HTTP requests to re-queue all created jobs.
///
/// This endpoint re-adds all jobs in Created status to their respective
/// processing queues based on their job types.
///
/// # Arguments
/// * `State(config)` - Shared application configuration
/// * `Query(filter)` - Optional job types filter from query parameters
///
/// # Returns
/// * `JobRouteResult` - Success response with job count and IDs, or error details
///
/// # Errors
/// * `JobRouteError::ProcessingError` - If the bulk requeue operation fails
async fn handle_requeue_created_jobs(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    // Check if admin is enabled
    if !config.server_config().admin_enabled {
        error!("Admin endpoints are disabled");
        return Err(JobRouteError::ProcessingError(
            "Admin endpoints are disabled. Enable with --admin-enabled flag.".to_string(),
        ));
    }

    info!(job_types = ?filter.job_type, "Admin: Requeue created jobs request received");

    match AdminService::requeue_created_jobs(filter.job_type.clone(), config.clone()).await {
        Ok(result) => {
            info!(
                success = result.success_count,
                failed = result.failed_count,
                job_types = ?filter.job_type,
                "Admin: Completed requeue of created jobs"
            );

            ORCHESTRATOR_METRICS.successful_job_operations.add(
                result.success_count as f64,
                &[KeyValue::new("operation_type", "admin_requeue_created"), KeyValue::new("admin", "true")],
            );

            if result.failed_count > 0 {
                ORCHESTRATOR_METRICS.failed_job_operations.add(
                    result.failed_count as f64,
                    &[KeyValue::new("operation_type", "admin_requeue_created"), KeyValue::new("admin", "true")],
                );
            }

            let message = if result.success_count == 0 && result.failed_count == 0 {
                "No created jobs to requeue".to_string()
            } else if result.failed_count > 0 {
                format!(
                    "Re-queued {} created job(s) for processing. {} job(s) failed to queue.",
                    result.success_count, result.failed_count
                )
            } else {
                format!("Successfully re-queued {} created job(s) for processing", result.success_count)
            };

            Ok(Json(ApiResponse::<BulkJobResponse>::success_with_data(
                BulkJobResponse {
                    success_count: result.success_count,
                    successful_job_ids: result.successful_job_ids,
                    failed_count: result.failed_count,
                    failed_job_ids: result.failed_job_ids,
                },
                Some(message),
            ))
            .into_response())
        }
        Err(e) => {
            error!(error = %e, job_types = ?filter.job_type, "Admin: Failed to requeue created jobs");
            ORCHESTRATOR_METRICS
                .failed_job_operations
                .add(1.0, &[KeyValue::new("operation_type", "admin_requeue_created"), KeyValue::new("admin", "true")]);
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

/// Creates a router for admin endpoints.
///
/// This function sets up all admin-related routes. These routes are only
/// functional when the admin_enabled flag is set in the configuration.
///
/// # Arguments
/// * `config` - Shared application configuration
///
/// # Returns
/// * `Router` - Configured router with all admin endpoints
///
/// # Available Endpoints
/// * `POST /jobs/retry/failed` - Retry all failed jobs (optionally filtered by job_type[])
/// * `POST /jobs/requeue/pending-verification` - Re-queue pending verification jobs
/// * `POST /jobs/requeue/created` - Re-queue created jobs
pub fn admin_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/jobs/retry/failed", post(handle_retry_all_failed_jobs))
        .route("/jobs/requeue/pending-verification", post(handle_requeue_pending_verification))
        .route("/jobs/requeue/created", post(handle_requeue_created_jobs))
        .with_state(config)
}
