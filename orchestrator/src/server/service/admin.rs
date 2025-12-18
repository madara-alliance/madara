use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::service::JobService;

/// Result of a bulk job operation
#[derive(Debug, Clone)]
pub struct BulkJobResult {
    pub success_count: u64,
    pub successful_job_ids: Vec<Uuid>,
    pub failed_count: u64,
    pub failed_job_ids: Vec<Uuid>,
}

/// Admin service for bulk job operations
pub struct AdminService;

impl AdminService {
    /// Internal helper: query jobs by status and process each with given function
    async fn process_jobs_by_status<F, Fut>(
        status: JobStatus,
        job_types: Vec<JobType>,
        config: Arc<Config>,
        op_name: &str,
        process_fn: F,
    ) -> Result<BulkJobResult, JobError>
    where
        F: Fn(Uuid, JobType, Arc<Config>) -> Fut,
        Fut: std::future::Future<Output = Result<(), JobError>>,
    {
        let jobs =
            config.database().get_jobs_by_types_and_statuses(job_types.clone(), vec![status.clone()], None).await?;
        info!(job_types = ?job_types, status = ?status, count = jobs.len(), "Admin: {} - found jobs", op_name);

        let mut successful_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();

        for job in &jobs {
            match process_fn(job.id, job.job_type.clone(), config.clone()).await {
                Ok(_) => {
                    successful_job_ids.push(job.id);
                    info!(job_id = %job.id, job_type = ?job.job_type, "Admin: {} - success", op_name);
                }
                Err(e) => {
                    failed_job_ids.push(job.id);
                    error!(job_id = %job.id, error = %e, "Admin: {} - failed", op_name);
                }
            }
        }

        let result = BulkJobResult {
            success_count: successful_job_ids.len() as u64,
            successful_job_ids,
            failed_count: failed_job_ids.len() as u64,
            failed_job_ids: failed_job_ids.clone(),
        };

        if result.failed_count > 0 {
            warn!(
                total = jobs.len(),
                success = result.success_count,
                failed = result.failed_count,
                "Admin: {} - completed with failures",
                op_name
            );
        }

        Ok(result)
    }

    pub async fn retry_all_failed_jobs(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        Self::process_jobs_by_status(
            JobStatus::Failed,
            job_types,
            config.clone(),
            "retry failed",
            |id, _, cfg| async move { JobHandlerService::retry_job(id, cfg).await },
        )
        .await
    }

    pub async fn retry_all_verification_timeout_jobs(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        Self::process_jobs_by_status(
            JobStatus::VerificationTimeout,
            job_types,
            config.clone(),
            "retry verification-timeout",
            |id, _, cfg| async move { JobHandlerService::retry_job(id, cfg).await },
        )
        .await
    }

    pub async fn requeue_pending_verification(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        Self::process_jobs_by_status(
            JobStatus::PendingVerification,
            job_types,
            config.clone(),
            "requeue pending-verification",
            |id, job_type, cfg| async move { JobService::add_job_to_verify_queue(cfg, id, &job_type, None).await },
        )
        .await
    }

    pub async fn requeue_created_jobs(job_types: Vec<JobType>, config: Arc<Config>) -> Result<BulkJobResult, JobError> {
        Self::process_jobs_by_status(
            JobStatus::Created,
            job_types,
            config.clone(),
            "requeue created",
            |id, job_type, cfg| async move { JobService::add_job_to_process_queue(id, &job_type, cfg).await },
        )
        .await
    }
}
