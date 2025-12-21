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

#[derive(Debug, Deserialize)]
pub struct JobTypeFilter {
    #[serde(default)]
    pub job_type: Vec<JobType>,
}

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

struct AdminOp {
    name: &'static str,
    metric_key: &'static str,
    empty_msg: &'static str,
    success_msg: &'static str,
}

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
        return Err(JobRouteError::ProcessingError(
            "Admin endpoints disabled. Enable with --admin-enabled flag.".to_string(),
        ));
    }

    info!(job_types = ?filter.job_type, "Admin: {}", op.name);

    match service_fn(filter.job_type.clone(), config).await {
        Ok(result) => {
            info!(success = result.success_count, failed = result.failed_count, "Admin: {} completed", op.name);

            ORCHESTRATOR_METRICS
                .successful_job_operations
                .add(result.success_count as f64, &[KeyValue::new("operation_type", op.metric_key)]);

            if result.failed_count > 0 {
                ORCHESTRATOR_METRICS
                    .failed_job_operations
                    .add(result.failed_count as f64, &[KeyValue::new("operation_type", op.metric_key)]);
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
            error!(error = %e, "Admin: {} failed", op.name);
            ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &[KeyValue::new("operation_type", op.metric_key)]);
            Err(JobRouteError::ProcessingError(e.to_string()))
        }
    }
}

async fn handle_retry_processing_failed(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    handle_admin_op(
        config,
        filter,
        AdminOp {
            name: "retry processing failed",
            metric_key: "admin_retry_processing_failed",
            empty_msg: "No processing failed jobs to retry",
            success_msg: "Retrying:",
        },
        AdminService::retry_all_processing_failed_jobs,
    )
    .await
}

async fn handle_retry_verification_failed(
    State(config): State<Arc<Config>>,
    Query(filter): Query<JobTypeFilter>,
) -> JobRouteResult {
    handle_admin_op(
        config,
        filter,
        AdminOp {
            name: "retry verification failed",
            metric_key: "admin_retry_verification_failed",
            empty_msg: "No verification failed jobs to retry",
            success_msg: "Retrying:",
        },
        AdminService::retry_all_verification_failed_jobs,
    )
    .await
}

pub fn admin_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/jobs/retry/processing-failed", post(handle_retry_processing_failed))
        .route("/jobs/retry/verification-failed", post(handle_retry_verification_failed))
        .with_state(config)
}
