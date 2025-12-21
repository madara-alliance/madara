use mockall_double::double;
use opentelemetry::KeyValue;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
#[double]
use crate::worker::event_handler::factory::factory;

pub struct JobService;

impl JobService {
    /// Retrieves a job by its ID from the database
    ///
    /// # Arguments
    /// * `id` - UUID of the job to retrieve
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<JobItem, JobError>` - The job if found, or JobNotFound error
    pub(crate) async fn get_job(id: Uuid, config: Arc<Config>) -> Result<JobItem, JobError> {
        config.database().get_job_by_id(id).await?.ok_or(JobError::JobNotFound { id })
    }

    /// Adds a job to its verification queue
    /// NOTE: In worker mode (now default), uses available_at timestamp instead of SQS delay
    pub async fn add_job_to_verify_queue(
        config: Arc<Config>,
        id: Uuid,
        job_type: &JobType,
        delay: Option<Duration>,
    ) -> Result<(), JobError> {
        // In worker mode, set available_at instead of using SQS delay
        if let Some(delay) = delay {
            let available_at = chrono::Utc::now()
                + chrono::Duration::from_std(delay)
                    .map_err(|e| JobError::Other(crate::error::other::OtherError::from(e.to_string())))?;

            // Update job with available_at for delayed verification
            let job = Self::get_job(id, config.clone()).await?;
            config
                .database()
                .update_job(
                    &job,
                    crate::types::jobs::job_updates::JobItemUpdates::new()
                        .update_available_at(Some(available_at))
                        .build(),
                )
                .await?;

            tracing::debug!(
                job_id = %id,
                delay_secs = delay.as_secs(),
                available_at = %available_at,
                "Worker mode: set available_at for delayed verification of {:?} job",
                job_type
            );
        } else {
            tracing::debug!(job_id = %id, "Worker mode: job ready for immediate verification of {:?} job", job_type);
        }
        Ok(())
    }

    /// Queues a job for verification by adding it to the verification queue
    ///
    /// # Arguments
    /// * `id` - UUID of the job to verify
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # Notes
    /// * Resets verification attempt count to 0
    /// * Sets appropriate delay for verification polling
    pub async fn queue_job_for_verification(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = Self::get_job(id, config.clone()).await?;
        let job_handler = factory::get_job_handler(&job.job_type).await;

        // Reset verification attempts and increment retry counter in common metadata
        job.metadata.common.verification_attempt_no = 0;
        job.metadata.common.verification_retry_attempt_no += 1;

        tracing::debug!(
            job_id = ?id,
            retry_count = job.metadata.common.verification_retry_attempt_no,
            "Incrementing verification retry attempt counter"
        );

        // Update job status and metadata
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new().update_status(JobStatus::Processed).update_metadata(job.metadata.clone()).build(),
            )
            .await?;

        // Add to verification queue with appropriate delay
        Self::add_job_to_verify_queue(
            config,
            id,
            &job.job_type,
            Some(Duration::from_secs(job_handler.verification_polling_delay_seconds())),
        )
        .await?;

        Ok(())
    }

    /// Moves a job to the ProcessingFailed state with the provided reason
    ///
    /// # Arguments
    /// * `job` - Reference to the job to mark as failed
    /// * `config` - Shared configuration
    /// * `reason` - Failure reason to record in metadata
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # Notes
    /// * Skips processing if job is already in ProcessingFailed status
    /// * Records failure reason in job metadata
    /// * Updates metrics for failed jobs
    pub async fn move_job_to_failed(job: &JobItem, config: Arc<Config>, reason: String) -> Result<(), JobError> {
        if job.status == JobStatus::Completed {
            tracing::error!(job_id = ?job.id, job_status = ?job.status, "Invalid state exists on DL queue");
            return Ok(());
        }
        // We assume that a Failure status will only show up if the message is sent twice from a queue
        // Can return silently because it's already been processed.
        else if job.status == JobStatus::ProcessingFailed {
            tracing::warn!(job_id = ?job.id, "Job already marked as failed, skipping processing");
            return Ok(());
        }

        let mut job_metadata = job.metadata.clone();
        let internal_id = &job.internal_id;

        tracing::debug!(job_id = ?job.id, "Updating job status to ProcessingFailed in database");
        // Update failure information in common metadata
        job_metadata.common.failure_reason = Some(reason.clone());

        // MULTI-ORCHESTRATOR FIX: Clear claim when job fails
        match config
            .database()
            .update_job(
                job,
                JobItemUpdates::new()
                    .update_status(JobStatus::ProcessingFailed)
                    .update_metadata(job_metadata)
                    .clear_claim() // Clear worker mode claim on failure
                    .build(),
            )
            .await
        {
            Ok(_) => {
                tracing::info!(
                    log_type = "completed",
                    category = "general",
                    function_type = "handle_job_failure",
                    block_no = %internal_id,
                    "General handle job failure completed for block"
                );
                ORCHESTRATOR_METRICS
                    .failed_jobs
                    .add(1.0, &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))]);

                // Send SNS alert for job failure
                let alert_message = format!(
                    "Job ProcessingFailed Alert: Job ID: {}, Type: {:?}, Block: {}, Reason: {}",
                    job.id, job.job_type, internal_id, reason
                );

                if let Err(e) = config.alerts().send_message(alert_message).await {
                    tracing::error!(
                        job_id = ?job.id,
                        error = ?e,
                        "Failed to send SNS alert for job failure"
                    );
                } else {
                    tracing::info!(
                        job_id = ?job.id,
                        "SNS alert sent successfully for job failure"
                    );
                }

                Ok(())
            }
            Err(e) => {
                tracing::error!(
                    log_type = "error",
                    category = "general",
                    function_type = "handle_job_failure",
                    block_no = %internal_id,
                    error = %e,
                    "General handle job failure failed for block"
                );
                Err(JobError::from(e))
            }
        }
    }
}
