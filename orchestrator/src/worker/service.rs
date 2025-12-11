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
use crate::types::queue::{QueueNameForJobType, QueueType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
#[double]
use crate::worker::event_handler::factory::factory;
use crate::worker::parser::job_queue_message::JobQueueMessage;

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

    /// Add a job into the queue with the given delay
    ///
    /// # Arguments
    /// * `config` - Shared configuration
    /// * `id` - UUID of the job to process
    /// * `queue` - Queue type to add the job to
    /// * `delay` - Optional delay for the job to be added to the queue
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    async fn add_job_to_queue(
        config: Arc<Config>,
        id: Uuid,
        queue: QueueType,
        delay: Option<Duration>,
    ) -> Result<(), JobError> {
        let message = JobQueueMessage { id };
        let payload = serde_json::to_string(&message)?;

        tracing::info!(
            queue = ?queue,
            job_id = %id,
            payload = %payload,
            delay_secs = ?delay.map(|d| d.as_secs()),
            "📤 Sending message to queue"
        );

        config.queue().send_message(queue.clone(), payload.clone(), delay).await.inspect_err(|e| {
            tracing::error!(
                queue = ?queue,
                job_id = %id,
                payload = %payload,
                error = ?e,
                "❌ Failed to send message to queue"
            );
        })?;

        tracing::info!(
            queue = ?queue,
            job_id = %id,
            "✅ Successfully sent message to queue"
        );
        Ok(())
    }

    /// Adds a job to its processing queue
    pub async fn add_job_to_process_queue(id: Uuid, job_type: &JobType, config: Arc<Config>) -> Result<(), JobError> {
        tracing::debug!(job_id = %id, "Adding a {:?} job to processing queue", job_type);
        Self::add_job_to_queue(config, id, job_type.process_queue_name(), None).await
    }

    /// Adds a job to its verification queue
    pub async fn add_job_to_verify_queue(
        config: Arc<Config>,
        id: Uuid,
        job_type: &JobType,
        delay: Option<Duration>,
    ) -> Result<(), JobError> {
        tracing::debug!(job_id = %id, "Adding a {:?} job to verification queue", job_type);
        Self::add_job_to_queue(config, id, job_type.verify_queue_name(), delay).await
    }

    /// Queues a job for processing by adding it to the process queue
    ///
    /// # Arguments
    /// * `id` - UUID of the job to process
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # State Transitions
    /// * Any valid state -> PendingProcess
    pub async fn queue_job_for_processing(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let job = Self::get_job(id, config.clone()).await?;

        // Add to process queue directly
        Self::add_job_to_process_queue(id, &job.job_type, config).await?;

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
                JobItemUpdates::new()
                    .update_status(JobStatus::PendingVerification)
                    .update_metadata(job.metadata.clone())
                    .build(),
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

    /// Moves a job to the Failed state with the provided reason
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
    /// * Skips processing if job is already in Failed status
    /// * Records failure reason in job metadata
    /// * Updates metrics for failed jobs
    pub async fn move_job_to_failed(job: &JobItem, config: Arc<Config>, reason: String) -> Result<(), JobError> {
        if job.status == JobStatus::Completed {
            tracing::error!(job_id = ?job.id, job_status = ?job.status, "Invalid state exists on DL queue");
            return Ok(());
        }
        // We assume that a Failure status will only show up if the message is sent twice from a queue
        // Can return silently because it's already been processed.
        else if job.status == JobStatus::Failed {
            tracing::warn!(job_id = ?job.id, "Job already marked as failed, skipping processing");
            return Ok(());
        }

        let mut job_metadata = job.metadata.clone();
        let internal_id = &job.internal_id;

        tracing::debug!(job_id = ?job.id, "Updating job status to Failed in database");
        // Update failure information in common metadata
        job_metadata.common.failure_reason = Some(reason.clone());

        match config
            .database()
            .update_job(
                job,
                JobItemUpdates::new().update_status(JobStatus::Failed).update_metadata(job_metadata).build(),
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
                    "Job Failed Alert: Job ID: {}, Type: {:?}, Block: {}, Reason: {}",
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
