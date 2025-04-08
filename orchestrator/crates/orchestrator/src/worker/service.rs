use crate::core::config::Config;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::{QueueNameForJobType, QueueType};
use crate::worker::event_handler::factory::JobFactory;
use crate::worker::parser::job_queue_message::JobQueueMessage;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

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
    async fn get_job(id: Uuid, config: Arc<Config>) -> Result<JobItem, JobError> {
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
        config.queue().send_message(queue.clone(), serde_json::to_string(&message)?, delay).await?;
        tracing::info!(
            log_type = "JobQueue",
            category = "add_job_to_queue",
            function_type = "add_job_to_queue",
            "Added job with id {:?} to {:?} queue",
            id,
            queue
        );
        Ok(())
    }
    pub async fn add_job_to_process_queue(id: Uuid, job_type: &JobType, config: Arc<Config>) -> Result<(), JobError> {
        tracing::info!("Adding job with id {:?} to processing queue", id);
        Self::add_job_to_queue(config, id, job_type.process_queue_name(), None).await
    }

    pub async fn add_job_to_verify_queue(
        config: Arc<Config>,
        id: Uuid,
        job_type: &JobType,
        delay: Option<Duration>,
    ) -> Result<(), JobError> {
        tracing::info!("Adding job with id {:?} to processing queue", id);
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
    #[tracing::instrument(skip(config), fields(category = "general"), ret, err)]
    pub async fn queue_job_for_processing(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let job = Self::get_job(id, config.clone()).await?;

        // Add to process queue directly
        Self::add_job_to_process_queue(id, &job.job_type, config).await?;

        Ok(())
    }

    /// Retries a failed job by reprocessing it.
    /// Only jobs with Failed status can be retried.
    ///
    /// # Arguments
    /// * `id` - UUID of the job to retry
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # State Transitions
    /// * `Failed` -> `PendingRetry` -> (normal processing flow)
    ///
    /// # Notes
    /// * Only jobs in Failed status can be retried
    /// * Transitions through PendingRetry status before normal processing
    /// * Uses standard process_job function after status update
    #[tracing::instrument(skip(config), fields(category = "general"), ret, err)]
    pub async fn retry_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = Self::get_job(id, config.clone()).await?;
        let internal_id = job.internal_id.clone();

        tracing::info!(
            log_type = "starting",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            "General retry job started for block"
        );

        if job.status != JobStatus::Failed {
            tracing::error!(
                job_id = ?id,
                status = ?job.status,
                "Cannot retry job: invalid status"
            );
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }

        // Increment the retry counter in common metadata
        job.metadata.common.process_retry_attempt_no += 1;
        job.metadata.common.process_attempt_no = 0;

        tracing::debug!(
            job_id = ?id,
            retry_count = job.metadata.common.process_retry_attempt_no,
            "Incrementing process retry attempt counter"
        );

        // Update job status and metadata to PendingRetry before processing
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::PendingRetry)
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await?;

        Self::add_job_to_process_queue(job.id, &job.job_type, config.clone()).await?;

        tracing::info!(
            log_type = "completed",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            "Successfully queued job for retry"
        );

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
    #[tracing::instrument(skip(config), fields(category = "general"), ret, err)]
    pub async fn queue_job_for_verification(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = Self::get_job(id, config.clone()).await?;
        let job_handler = JobFactory::get_job_handler(&job.job_type).await;

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
}
