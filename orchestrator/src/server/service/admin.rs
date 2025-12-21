use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::service::JobService;

#[derive(Debug, Clone)]
pub struct BulkJobResult {
    pub success_count: u64,
    pub successful_job_ids: Vec<Uuid>,
    pub failed_count: u64,
    pub failed_job_ids: Vec<Uuid>,
}

pub struct AdminService;

impl AdminService {
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
        info!(status = ?status, count = jobs.len(), "Admin: {} - found jobs", op_name);

        let mut successful_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();

        for job in &jobs {
            match process_fn(job.id, job.job_type.clone(), config.clone()).await {
                Ok(_) => successful_job_ids.push(job.id),
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
            failed_job_ids,
        };

        if result.failed_count > 0 {
            warn!(
                success = result.success_count,
                failed = result.failed_count,
                "Admin: {} - completed with failures",
                op_name
            );
        }

        Ok(result)
    }

    /// Retry all ProcessingFailed jobs -> moves them to PendingRetryProcessing
    pub async fn retry_all_processing_failed_jobs(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        Self::process_jobs_by_status(
            JobStatus::ProcessingFailed,
            job_types,
            config,
            "retry processing failed",
            |id, _, cfg| async move { JobHandlerService::retry_job(id, cfg).await },
        )
        .await
    }

    /// Retry all VerificationFailed jobs -> moves them to Processed for re-verification
    pub async fn retry_all_verification_failed_jobs(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        Self::process_jobs_by_status(
            JobStatus::VerificationFailed,
            job_types,
            config,
            "retry verification failed",
            |id, _, cfg| async move {
                let job = JobService::get_job(id, cfg.clone()).await?;
                cfg.database()
                    .update_job(&job, JobItemUpdates::new().update_status(JobStatus::Processed).build())
                    .await?;
                Ok(())
            },
        )
        .await
    }
}
