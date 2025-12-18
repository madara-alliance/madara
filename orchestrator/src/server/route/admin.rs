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
use super::super::service::admin::{AdminService, BulkJobResult};
use super::super::types::{ApiResponse, JobRouteResult};
use crate::core::config::Config;
use crate::types::jobs::types::JobType;
use crate::utils::metrics::ORCHESTRATOR_METRICS;

/// Query parameters for filtering jobs by types
#[derive(Debug, Deserialize)]
pub struct JobTypeFilter {
    #[serde(default)]
    pub job_type: Vec<JobType>,
}

/// Response for bulk job operations
#[derive(Debug, Serialize, Deserialize)]
pub struct BulkJobResponse {
    pub success_count: u64,
    pub successful_job_ids: Vec<Uuid>,
    pub failed_count: u64,
    pub failed_job_ids: Vec<Uuid>,
}

impl From<BulkJobResult> for BulkJobResponse {
    fn from(r: BulkJobResult) -> Self {
        Self {
            success_count: r.success_count,
            successful_job_ids: r.successful_job_ids,
            failed_count: r.failed_count,
            failed_job_ids: r.failed_job_ids,
        }
    }
}

/// Admin operation descriptor for generic handler
struct AdminOp {
    name: &'static str,
    metric_key: &'static str,
    empty_msg: &'static str,
    success_msg: &'static str,
}

/// Generic admin handler that reduces code duplication
async fn handle_admin_op<F, Fut>(
    config: Arc<Config>,
    filter: JobTypeFilter,
    op: AdminOp,
    service_fn: F,
) -> JobRouteResult
where
    F: FnOnce(Vec<JobType>, Arc<Config>) -> Fut,
    Fut: std::future::Future<Output = Result<BulkJobResult, crate::error::job::JobError>>,
{
    if !config.server_config().admin_enabled {
        error!("Admin endpoints are disabled");
        return Err(JobRouteError::ProcessingError(
            "Admin endpoints are disabled. Enable with --admin-enabled flag.".to_string(),
        ));
    }

    info!(job_types = ?filter.job_type, "Admin: {} request received", op.name);

    match service_fn(filter.job_type.clone(), config).await {
        Ok(result) => {
            info!(success = result.success_count, failed = result.failed_count, job_types = ?filter.job_type, "Admin: Completed {}", op.name);

            ORCHESTRATOR_METRICS.successful_job_operations.add(
                result.success_count as f64,
                &[KeyValue::new("operation_type", op.metric_key), KeyValue::new("admin", "true")],
            );

            if result.failed_count > 0 {
                ORCHESTRATOR_METRICS.failed_job_operations.add(
                    result.failed_count as f64,
                    &[KeyValue::new("operation_type", op.metric_key), KeyValue::new("admin", "true")],
                );
            }

            let message = if result.success_count == 0 && result.failed_count == 0 {
                op.empty_msg.to_string()
            } else if result.failed_count > 0 {
                format!("{} {} job(s). {} failed.", op.success_msg, result.success_count, result.failed_count)
            } else {
                format!("{} {} job(s)", op.success_msg, result.success_count)
            };

            Ok(Json(ApiResponse::success_with_data(BulkJobResponse::from(result), Some(message))).into_response())
        }
        Err(e) => {
            error!(error = %e, job_types = ?filter.job_type, "Admin: Failed {}", op.name);
            ORCHESTRATOR_METRICS
                .failed_job_operations
                .add(1.0, &[KeyValue::new("operation_type", op.metric_key), KeyValue::new("admin", "true")]);
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

async fn handle_retry_all_failed_jobs(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    handle_admin_op(
        config,
        filter,
        AdminOp {
            name: "retry failed jobs",
            metric_key: "admin_retry_failed",
            empty_msg: "No failed jobs to retry",
            success_msg: "Queued for retry:",
        },
        AdminService::retry_all_failed_jobs,
    )
    .await
}

async fn handle_retry_verification_timeout_jobs(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    handle_admin_op(
        config,
        filter,
        AdminOp {
            name: "retry verification-timeout jobs",
            metric_key: "admin_retry_verification_timeout",
            empty_msg: "No verification-timeout jobs to retry",
            success_msg: "Queued for retry:",
        },
        AdminService::retry_all_verification_timeout_jobs,
    )
    .await
}

/// # Warning
/// **RECOVERY ENDPOINT** - Blindly re-queues ALL PendingVerification jobs, including those
/// already in queue. May cause duplicate messages. Use only after queue consumer crashes.
async fn handle_requeue_pending_verification(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    handle_admin_op(
        config,
        filter,
        AdminOp {
            name: "requeue pending verification jobs",
            metric_key: "admin_requeue_verification",
            empty_msg: "No pending verification jobs to requeue",
            success_msg: "Re-queued for verification:",
        },
        AdminService::requeue_pending_verification,
    )
    .await
}

/// # Warning
/// **RECOVERY ENDPOINT** - Blindly re-queues ALL Created jobs, including those
/// already in queue. May cause duplicate messages. Use only after queue consumer crashes.
async fn handle_requeue_created_jobs(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    handle_admin_op(
        config,
        filter,
        AdminOp {
            name: "requeue created jobs",
            metric_key: "admin_requeue_created",
            empty_msg: "No created jobs to requeue",
            success_msg: "Re-queued for processing:",
        },
        AdminService::requeue_created_jobs,
    )
    .await
}

pub fn admin_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/jobs/retry/failed", post(handle_retry_all_failed_jobs))
        .route("/jobs/retry/verification-timeout", post(handle_retry_verification_timeout_jobs))
        .route("/jobs/requeue/pending-verification", post(handle_requeue_pending_verification))
        .route("/jobs/requeue/created", post(handle_requeue_created_jobs))
        .with_state(config)
}
