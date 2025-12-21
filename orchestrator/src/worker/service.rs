use opentelemetry::KeyValue;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::JobStatus;
use crate::utils::metrics::ORCHESTRATOR_METRICS;

pub struct JobService;

impl JobService {
    /// Retrieves a job by its ID from the database
    pub(crate) async fn get_job(id: Uuid, config: Arc<Config>) -> Result<JobItem, JobError> {
        config.database().get_job_by_id(id).await?.ok_or(JobError::JobNotFound { id })
    }

    /// Requeues a job for verification (used by API endpoint for manual re-verification)
    pub async fn requeue_for_verification(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = Self::get_job(id, config.clone()).await?;

        // Reset verification attempts and increment retry counter
        job.metadata.common.verification_attempt_no = 0;
        job.metadata.common.verification_retry_attempt_no += 1;

        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new().update_status(JobStatus::Processed).update_metadata(job.metadata.clone()).build(),
            )
            .await?;

        tracing::info!(job_id = %id, "Requeued job for verification");
        Ok(())
    }

    /// Marks a job as ProcessingFailed with the given reason
    pub async fn move_job_to_failed(job: &JobItem, config: Arc<Config>, reason: String) -> Result<(), JobError> {
        if job.status == JobStatus::Completed {
            tracing::error!(job_id = ?job.id, "Completed job on failure queue - invalid state");
            return Ok(());
        }
        if job.status == JobStatus::ProcessingFailed {
            tracing::warn!(job_id = ?job.id, "Job already failed, skipping");
            return Ok(());
        }

        let mut metadata = job.metadata.clone();
        metadata.common.failure_reason = Some(reason.clone());

        config
            .database()
            .update_job(
                job,
                JobItemUpdates::new()
                    .update_status(JobStatus::ProcessingFailed)
                    .update_metadata(metadata)
                    .clear_claim()
                    .build(),
            )
            .await?;

        tracing::info!("Job {} marked as ProcessingFailed", job.internal_id);

        ORCHESTRATOR_METRICS
            .failed_jobs
            .add(1.0, &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))]);

        // Send alert
        let alert_message = format!(
            "Job ProcessingFailed: ID={}, Type={:?}, Block={}, Reason={}",
            job.id, job.job_type, job.internal_id, reason
        );
        if let Err(e) = config.alerts().send_message(alert_message).await {
            tracing::error!(job_id = ?job.id, error = ?e, "Failed to send failure alert");
        }

        Ok(())
    }
}
