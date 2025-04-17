use std::collections::HashMap;
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use color_eyre::eyre::{eyre, Context};
use conversion::parse_string;
use da_job::DaError;
use futures::FutureExt;
use mockall::automock;
use mockall_double::double;
use opentelemetry::KeyValue;
use proving_job::ProvingError;
use snos_job::error::FactError;
use snos_job::SnosError;
use state_update_job::StateUpdateError;
use types::{ExternalId, JobItemUpdates};
use uuid::Uuid;

use crate::config::Config;
use crate::helpers::JobProcessingState;
#[double]
use crate::jobs::job_handler_factory::factory;
use crate::jobs::metadata::JobMetadata;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::queue::job_queue::{add_job_to_process_queue, add_job_to_verification_queue, ConsumptionError};

pub mod conversion;
pub mod da_job;
pub mod job_handler_factory;
pub mod metadata;
pub mod proving_job;
pub mod register_proof_job;
pub mod snos_job;
pub mod state_update_job;
pub mod types;
use thiserror::Error;

/// Error types for job-related operations in the orchestrator
#[derive(Error, Debug, PartialEq)]
pub enum JobError {
    /// Indicates an invalid job ID was provided
    #[error("Job id {id:?} is invalid.")]
    InvalidId { id: String },

    /// Indicates an attempt to create a duplicate job
    #[error("Job already exists for internal_id {internal_id:?} and job_type {job_type:?}. Skipping!")]
    JobAlreadyExists { internal_id: String, job_type: JobType },

    /// Indicates the job is in an invalid status for the requested operation
    #[error("Invalid status {job_status:?} for job with id {id:?}. Cannot process.")]
    InvalidStatus { id: Uuid, job_status: JobStatus },

    /// Indicates the requested job could not be found
    #[error("Failed to find job with id {id:?}")]
    JobNotFound { id: Uuid },

    /// Indicates a metadata counter would overflow if incremented
    #[error("Incrementing key {} in metadata would exceed u64::MAX", key)]
    KeyOutOfBounds { key: String },

    /// Wraps errors from DA layer operations
    #[error("DA Error: {0}")]
    DaJobError(#[from] DaError),

    /// Wraps errors from proving operations
    #[error("Proving Error: {0}")]
    ProvingJobError(#[from] ProvingError),

    /// Wraps errors from state update operations
    #[error("Proving Error: {0}")]
    StateUpdateJobError(#[from] StateUpdateError),

    /// Wraps errors from SNOS operations
    #[error("Snos Error: {0}")]
    SnosJobError(#[from] SnosError),

    /// Wraps errors from queue handling operations
    #[error("Queue Handling Error: {0}")]
    ConsumptionError(#[from] ConsumptionError),

    /// Wraps errors from fact operations
    #[error("Fact Error: {0}")]
    FactError(#[from] FactError),

    /// Wraps general errors that don't fit other categories
    #[error("Other error: {0}")]
    Other(#[from] OtherError),

    /// Indicates that the maximum capacity of jobs currently being processed has been reached
    #[error("Max Capacity Reached, Already processing")]
    MaxCapacityReached,

    /// Indicates an error occurred while extracting the processing lock
    #[error("Error extracting processing lock: {0}")]
    LockError(String),
}

/// Wrapper Type for Other(<>) job type
///
/// Provides a generic error type for cases that don't fit into specific error categories
/// while maintaining error chain context.
#[derive(Debug)]
pub struct OtherError(color_eyre::eyre::Error);

impl fmt::Display for OtherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for OtherError {}

impl PartialEq for OtherError {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl From<color_eyre::eyre::Error> for OtherError {
    fn from(err: color_eyre::eyre::Error) -> Self {
        OtherError(err)
    }
}

impl From<String> for OtherError {
    fn from(error_string: String) -> Self {
        OtherError(eyre!(error_string))
    }
}

impl From<color_eyre::Report> for JobError {
    fn from(err: color_eyre::Report) -> Self {
        JobError::Other(OtherError(err))
    }
}

/// Job Trait
///
/// The Job trait is used to define the methods that a job
/// should implement to be used as a job for the orchestrator. The orchestrator automatically
/// handles queueing and processing of jobs as long as they implement the trait.
///
/// # Implementation Requirements
/// Implementors must be both `Send` and `Sync` to work with the async processing system.
#[automock]
#[async_trait]
pub trait Job: Send + Sync {
    /// Should build a new job item and return it
    ///
    /// # Arguments
    /// * `config` - Shared configuration for the job
    /// * `internal_id` - Unique identifier for internal tracking
    /// * `metadata` - Additional key-value pairs associated with the job
    ///
    /// # Returns
    /// * `Result<JobItem, JobError>` - The created job item or an error
    async fn create_job(
        &self,
        config: Arc<Config>,
        internal_id: String,
        metadata: JobMetadata,
    ) -> Result<JobItem, JobError>;

    /// Should process the job and return the external_id which can be used to
    /// track the status of the job. For example, a DA job will submit the state diff
    /// to the DA layer and return the txn hash.
    ///
    /// # Arguments
    /// * `config` - Shared configuration for the job
    /// * `job` - Mutable reference to the job being processed
    ///
    /// # Returns
    /// * `Result<String, JobError>` - External tracking ID or an error
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError>;

    /// Should verify the job and return the status of the verification. For example,
    /// a DA job will verify the inclusion of the state diff in the DA layer and return
    /// the status of the verification.
    ///
    /// # Arguments
    /// * `config` - Shared configuration for the job
    /// * `job` - Mutable reference to the job being verified
    ///
    /// # Returns
    /// * `Result<JobVerificationStatus, JobError>` - Current verification status or an error
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError>;

    /// Should return the maximum number of attempts to process the job. A new attempt is made
    /// every time the verification returns `JobVerificationStatus::Rejected`
    fn max_process_attempts(&self) -> u16;

    /// Should return the maximum number of attempts to verify the job. A new attempt is made
    /// every few seconds depending on the result `verification_polling_delay_seconds`
    fn max_verification_attempts(&self) -> u16;

    /// Should return the number of seconds to wait before polling for verification
    fn verification_polling_delay_seconds(&self) -> u64;

    fn job_processing_lock(&self, config: Arc<Config>) -> Option<Arc<JobProcessingState>>;
}

/// Creates the job in the DB in the created state and adds it to the process queue
///
/// # Arguments
/// * `job_type` - Type of job to create
/// * `internal_id` - Unique identifier for internal tracking
/// * `metadata` - Additional key-value pairs for the job
/// * `config` - Shared configuration
///
/// # Returns
/// * `Result<(), JobError>` - Success or an error
///
/// # Metrics
/// * Records block gauge
/// * Updates successful job operations count
/// * Records job response time
///
/// # Notes
/// * Skips creation if job already exists with same internal_id and job_type
/// * Automatically adds the job to the process queue upon successful creation
#[tracing::instrument(fields(category = "general"), skip(config), ret, err)]
pub async fn create_job(
    job_type: JobType,
    internal_id: String,
    metadata: JobMetadata,
    config: Arc<Config>,
) -> Result<(), JobError> {
    let start = Instant::now();
    tracing::info!(
        log_type = "starting",
        category = "general",
        function_type = "create_job",
        job_type = ?job_type,
        block_no = %internal_id,
        "General create job started for block"
    );

    tracing::debug!(
        job_type = ?job_type,
        internal_id = %internal_id,
        metadata = ?metadata,
        "Job creation details"
    );

    let existing_job = config
        .database()
        .get_job_by_internal_id_and_type(internal_id.as_str(), &job_type)
        .await
        .map_err(|e| JobError::Other(OtherError(e)))?;

    if existing_job.is_some() {
        tracing::warn!("{}", JobError::JobAlreadyExists { internal_id, job_type });
        return Ok(());
    }

    let job_handler = factory::get_job_handler(&job_type).await;
    let job_item = job_handler.create_job(config.clone(), internal_id.clone(), metadata).await?;
    config.database().create_job_item(job_item.clone()).await?;
    println!("Job item inside the create job function: {:?}", job_item);
    add_job_to_process_queue(job_item.id, &job_type, config.clone())
        .await
        .map_err(|e| JobError::Other(OtherError(e)))?;

    let attributes =
        [KeyValue::new("operation_job_type", format!("{:?}", job_type)), KeyValue::new("operation_type", "create_job")];

    tracing::info!(
        log_type = "completed",
        category = "general",
        function_type = "create_job",
        block_no = %internal_id,
        "General create job completed for block"
    );

    let duration = start.elapsed();
    ORCHESTRATOR_METRICS.block_gauge.record(parse_string(&internal_id)?, &attributes);
    ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
    ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
    Ok(())
}

/// Processes the job, increments the process attempt count and updates the status of the job in the
/// DB. It then adds the job to the verification queue.
///
/// # Arguments
/// * `id` - UUID of the job to process
/// * `config` - Shared configuration
///
/// # Returns
/// * `Result<(), JobError>` - Success or an error
///
/// # State Transitions
/// * `Created` -> `LockedForProcessing` -> `PendingVerification`
/// * `VerificationFailed` -> `LockedForProcessing` -> `PendingVerification`
/// * `PendingRetry` -> `LockedForProcessing` -> `PendingVerification`
///
/// # Metrics
/// * Updates block gauge
/// * Records successful job operations
/// * Tracks job response time
///
/// # Notes
/// * Only processes jobs in Created, VerificationFailed, or PendingRetry status
/// * Updates job version to prevent concurrent processing
/// * Adds processing completion timestamp to metadata
/// * Automatically adds job to verification queue upon successful processing
#[tracing::instrument(skip(config), fields(category = "general", job, job_type, internal_id), ret, err)]
pub async fn process_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let start = Instant::now();
    let mut job = get_job(id, config.clone()).await?;
    let internal_id = job.internal_id.clone();
    tracing::info!(
        log_type = "starting",
        category = "general",
        function_type = "process_job",
        block_no = %internal_id,
        "General process job started for block"
    );

    tracing::Span::current().record("job", format!("{:?}", job.clone()));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type));
    tracing::Span::current().record("internal_id", job.internal_id.clone());

    tracing::debug!(job_id = ?id, status = ?job.status, "Current job status");
    match job.status {
        // we only want to process jobs that are in the created or verification failed state or if it's been called from
        // the retry endpoint (in this case it would be PendingRetry status) verification failed state means
        // that the previous processing failed and we want to retry
        JobStatus::Created | JobStatus::VerificationFailed | JobStatus::PendingRetry => {
            tracing::info!(job_id = ?id, status = ?job.status, "Processing job");
        }
        _ => {
            tracing::warn!(job_id = ?id, status = ?job.status, "Cannot process job with current status");
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }
    }

    let job_handler = factory::get_job_handler(&job.job_type).await;
    let job_processing_locks = job_handler.job_processing_lock(config.clone());

    let permit = if let Some(ref processing_locks) = job_processing_locks {
        Some(processing_locks.try_acquire_lock(&job, config.clone()).await?)
    } else {
        None
    };

    // this updates the version of the job. this ensures that if another thread was about to process
    // the same job, it would fail to update the job in the database because the version would be
    // outdated
    tracing::debug!(job_id = ?id, "Updating job status to LockedForProcessing");
    job.metadata.common.process_started_at = Some(Utc::now());
    let mut job = config
        .database()
        .update_job(
            &job,
            JobItemUpdates::new()
                .update_status(JobStatus::LockedForProcessing)
                .update_metadata(job.metadata.clone())
                .build(),
        )
        .await
        .map_err(|e| {
            tracing::error!(job_id = ?id, error = ?e, "Failed to update job status");
            JobError::Other(OtherError(e))
        })?;

    tracing::debug!(job_id = ?id, job_type = ?job.job_type, "Getting job handler");
    let external_id = match AssertUnwindSafe(job_handler.process_job(config.clone(), &mut job)).catch_unwind().await {
        Ok(Ok(external_id)) => {
            tracing::debug!(job_id = ?id, "Successfully processed job");
            // Add the time of processing to the metadata.
            job.metadata.common.process_completed_at = Some(Utc::now());

            external_id
        }
        Ok(Err(e)) => {
            // TODO: I think most of the times the errors will not be fixed automatically
            // if we just retry. But for some failures like DB issues, it might be possible
            // that retrying will work. So we can add a retry logic here to improve robustness.
            tracing::error!(job_id = ?id, error = ?e, "Failed to process job");
            return move_job_to_failed(&job, config.clone(), format!("Processing failed: {}", e)).await;
        }
        Err(panic) => {
            let panic_msg = panic
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("Unknown panic message");

            tracing::error!(job_id = ?id, panic_msg = %panic_msg, "Job handler panicked during processing");
            return move_job_to_failed(
                &job,
                config.clone(),
                format!("Job handler panicked with message: {}", panic_msg),
            )
            .await;
        }
    };

    // Increment process attempt counter
    job.metadata.common.process_attempt_no += 1;

    // Update job status and metadata
    tracing::debug!(job_id = ?id, "Updating job status to PendingVerification");
    config
        .database()
        .update_job(
            &job,
            JobItemUpdates::new()
                .update_status(JobStatus::PendingVerification)
                .update_metadata(job.metadata.clone())
                .update_external_id(external_id.clone().into())
                .build(),
        )
        .await
        .map_err(|e| {
            tracing::error!(job_id = ?id, error = ?e, "Failed to update job status");
            JobError::Other(OtherError(e))
        })?;

    // Add to verification queue
    tracing::debug!(job_id = ?id, "Adding job to verification queue");
    add_job_to_verification_queue(
        job.id,
        &job.job_type,
        Duration::from_secs(job_handler.verification_polling_delay_seconds()),
        config.clone(),
    )
    .await
    .map_err(|e| {
        tracing::error!(job_id = ?id, error = ?e, "Failed to add job to verification queue");
        JobError::Other(OtherError(e))
    })?;

    let attributes = vec![
        KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        KeyValue::new("operation_type", "process_job"),
    ];

    tracing::info!(
        log_type = "completed",
        category = "general",
        function_type = "process_job",
        block_no = %internal_id,
        "General process job completed for block"
    );

    let duration = start.elapsed();
    ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
    ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
    register_block_gauge(job.job_type, &job.internal_id, external_id.into(), &attributes)?;

    if let Some(permit) = permit {
        if let Some(ref processing_locks) = job_processing_locks {
            processing_locks.try_release_lock(permit).await?;
        }
    }

    Ok(())
}

/// Verify Job Function
///
/// Verifies the job and updates the status of the job in the DB. If the verification fails, it
/// retries processing the job if the max attempts have not been exceeded. If the max attempts have
/// been exceeded, it marks the job as timed out. If the verification is still pending, it pushes
/// the job back to the queue.
///
/// # Arguments
/// * `id` - UUID of the job to verify
/// * `config` - Shared configuration
///
/// # Returns
/// * `Result<(), JobError>` - Success or an error
///
/// # State Transitions
/// * `PendingVerification` -> `Completed` (on successful verification)
/// * `PendingVerification` -> `VerificationFailed` (on verification rejection)
/// * `PendingVerification` -> `VerificationTimeout` (max attempts reached)
///
/// # Metrics
/// * Records verification time if processing completion timestamp exists
/// * Updates block gauge and job operation metrics
/// * Tracks successful operations and response time
///
/// # Notes
/// * Only jobs in `PendingVerification` or `VerificationTimeout` status can be verified
/// * Automatically retries processing if verification fails and max attempts not reached
/// * Removes processing_finished_at from metadata upon successful verification
#[tracing::instrument(
    skip(config),
    fields(category = "general", job, job_type, internal_id, verification_status),
    ret,
    err
)]
pub async fn verify_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let start = Instant::now();
    let mut job = get_job(id, config.clone()).await?;
    let internal_id = job.internal_id.clone();
    tracing::info!(log_type = "starting", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job started for block");

    tracing::Span::current().record("job", format!("{:?}", job.clone()));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type.clone()));
    tracing::Span::current().record("internal_id", job.internal_id.clone());

    match job.status {
        // Jobs with `VerificationTimeout` will be retired manually after resetting verification attempt number to 0.
        JobStatus::PendingVerification | JobStatus::VerificationTimeout => {
            tracing::info!(job_id = ?id, status = ?job.status, "Proceeding with verification");
        }
        _ => {
            tracing::error!(job_id = ?id, status = ?job.status, "Invalid job status for verification");
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }
    }

    let job_handler = factory::get_job_handler(&job.job_type).await;
    tracing::debug!(job_id = ?id, "Verifying job with handler");

    job.metadata.common.verification_started_at = Some(Utc::now());
    let mut job = config
        .database()
        .update_job(&job, JobItemUpdates::new().update_metadata(job.metadata.clone()).build())
        .await
        .map_err(|e| {
            tracing::error!(job_id = ?id, error = ?e, "Failed to update job status");
            JobError::Other(OtherError(e))
        })?;

    let verification_status = job_handler.verify_job(config.clone(), &mut job).await?;
    tracing::Span::current().record("verification_status", format!("{:?}", &verification_status));

    let mut attributes = vec![
        KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        KeyValue::new("operation_type", "verify_job"),
        KeyValue::new("operation_verification_status", format!("{:?}", &verification_status)),
    ];
    let mut operation_job_status: Option<JobStatus> = None;

    match verification_status {
        JobVerificationStatus::Verified => {
            tracing::info!(job_id = ?id, "Job verified successfully");
            // Calculate verification time if processing completion timestamp exists
            if let Some(verification_time) = job.metadata.common.verification_started_at {
                let time_taken = (Utc::now() - verification_time).num_milliseconds();
                ORCHESTRATOR_METRICS
                    .verification_time
                    .record(time_taken as f64, &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))]);
            } else {
                tracing::warn!("Failed to calculate verification time: Missing processing completion timestamp");
            }

            // Update verification completed timestamp and update status
            job.metadata.common.verification_completed_at = Some(Utc::now());
            config
                .database()
                .update_job(
                    &job,
                    JobItemUpdates::new()
                        .update_metadata(job.metadata.clone())
                        .update_status(JobStatus::Completed)
                        .build(),
                )
                .await
                .map_err(|e| {
                    tracing::error!(job_id = ?id, error = ?e, "Failed to update job status to Completed");
                    JobError::Other(OtherError(e))
                })?;
            operation_job_status = Some(JobStatus::Completed);
        }
        JobVerificationStatus::Rejected(e) => {
            tracing::error!(job_id = ?id, error = ?e, "Job verification rejected");

            // Update metadata with error information
            job.metadata.common.failure_reason = Some(e.clone());
            operation_job_status = Some(JobStatus::VerificationFailed);

            if job.metadata.common.process_attempt_no < job_handler.max_process_attempts() {
                tracing::info!(
                    job_id = ?id,
                    attempt = job.metadata.common.process_attempt_no + 1,
                    "Verification failed. Retrying job processing"
                );

                config
                    .database()
                    .update_job(
                        &job,
                        JobItemUpdates::new()
                            .update_status(JobStatus::VerificationFailed)
                            .update_metadata(job.metadata.clone())
                            .build(),
                    )
                    .await
                    .map_err(|e| {
                        tracing::error!(job_id = ?id, error = ?e, "Failed to update job status to VerificationFailed");
                        JobError::Other(OtherError(e))
                    })?;
                add_job_to_process_queue(job.id, &job.job_type, config.clone())
                    .await
                    .map_err(|e| JobError::Other(OtherError(e)))?;
            } else {
                tracing::warn!(job_id = ?id, "Max process attempts reached. Job will not be retried");
                return move_job_to_failed(
                    &job,
                    config.clone(),
                    format!(
                        "Verification rejected. Max process attempts reached: {}",
                        job.metadata.common.process_attempt_no
                    ),
                )
                .await;
            }
        }
        JobVerificationStatus::Pending => {
            tracing::debug!(job_id = ?id, "Job verification still pending");

            if job.metadata.common.verification_attempt_no >= job_handler.max_verification_attempts() {
                tracing::warn!(job_id = ?id, "Max verification attempts reached. Marking job as timed out");
                config
                    .database()
                    .update_job(&job, JobItemUpdates::new().update_status(JobStatus::VerificationTimeout).build())
                    .await
                    .map_err(|e| {
                        tracing::error!(job_id = ?id, error = ?e, "Failed to update job status to VerificationTimeout");
                        JobError::Other(OtherError(e))
                    })?;
                operation_job_status = Some(JobStatus::VerificationTimeout);
            } else {
                // Increment verification attempts
                job.metadata.common.verification_attempt_no += 1;

                config
                    .database()
                    .update_job(&job, JobItemUpdates::new().update_metadata(job.metadata.clone()).build())
                    .await
                    .map_err(|e| {
                        tracing::error!(job_id = ?id, error = ?e, "Failed to update job metadata");
                        JobError::Other(OtherError(e))
                    })?;

                tracing::debug!(job_id = ?id, "Adding job back to verification queue");
                add_job_to_verification_queue(
                    job.id,
                    &job.job_type,
                    Duration::from_secs(job_handler.verification_polling_delay_seconds()),
                    config.clone(),
                )
                .await
                .map_err(|e| {
                    tracing::error!(job_id = ?id, error = ?e, "Failed to add job to verification queue");
                    JobError::Other(OtherError(e))
                })?;
            }
        }
    };

    if let Some(job_status) = operation_job_status {
        attributes.push(KeyValue::new("operation_job_status", format!("{}", job_status)));
    }

    tracing::info!(log_type = "completed", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job completed for block");
    let duration = start.elapsed();
    ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
    ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
    register_block_gauge(job.job_type, &job.internal_id, job.external_id, &attributes)?;
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
    let mut job = get_job(id, config.clone()).await?;
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
    // Reset the process attempt counter to 0, to ensure a fresh start
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
            JobItemUpdates::new().update_status(JobStatus::PendingRetry).update_metadata(job.metadata.clone()).build(),
        )
        .await
        .map_err(|e| {
            tracing::error!(
                job_id = ?id,
                error = ?e,
                "Failed to update job status to PendingRetry"
            );
            JobError::Other(OtherError(e))
        })?;

    add_job_to_process_queue(job.id, &job.job_type, config.clone()).await.map_err(|e| {
        tracing::error!(
            log_type = "error",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            error = %e,
            "Failed to add job to process queue"
        );
        JobError::Other(OtherError(e))
    })?;

    tracing::info!(
        log_type = "completed",
        category = "general",
        function_type = "retry_job",
        block_no = %internal_id,
        "Successfully queued job for retry"
    );

    Ok(())
}

/// Terminates the job and updates the status of the job in the DB.
///
/// # Arguments
/// * `id` - UUID of the job to handle failure for
/// * `config` - Shared configuration
///
/// # Returns
/// * `Result<(), JobError>` - Success or an error
///
/// # Notes
/// * Logs error if the job status `Completed` is existing on DL queue
/// * Updates job status to Failed and records failure reason in metadata
/// * Updates metrics for failed jobs
#[tracing::instrument(skip(config), fields(job_status, job_type), ret, err)]
pub async fn handle_job_failure(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let job = get_job(id, config.clone()).await?.clone();
    let internal_id = job.internal_id.clone();
    tracing::info!(log_type = "starting", category = "general", function_type = "handle_job_failure", block_no = %internal_id, "General handle job failure started for block");

    tracing::Span::current().record("job_status", format!("{:?}", job.status));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type));

    tracing::debug!(job_id = ?id, job_status = ?job.status, job_type = ?job.job_type, block_no = %internal_id, "Job details for failure handling for block");
    let status = job.status.clone().to_string();
    move_job_to_failed(&job, config.clone(), format!("Received failure queue message for job with status: {}", status))
        .await
}

fn register_block_gauge(
    job_type: JobType,
    internal_id: &str,
    external_id: ExternalId,
    attributes: &[KeyValue],
) -> Result<(), JobError> {
    let block_number = if let JobType::StateTransition = job_type {
        parse_string(
            external_id
                .unwrap_string()
                .map_err(|e| JobError::Other(OtherError::from(format!("Could not parse string: {e}"))))?,
        )
    } else {
        parse_string(internal_id)
    }?;

    ORCHESTRATOR_METRICS.block_gauge.record(block_number, attributes);
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
async fn move_job_to_failed(job: &JobItem, config: Arc<Config>, reason: String) -> Result<(), JobError> {
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
    let internal_id = job.internal_id.clone();

    tracing::debug!(job_id = ?job.id, "Updating job status to Failed in database");
    // Update failure information in common metadata
    job_metadata.common.failure_reason = Some(reason);

    match config
        .database()
        .update_job(job, JobItemUpdates::new().update_status(JobStatus::Failed).update_metadata(job_metadata).build())
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
            Err(JobError::Other(OtherError(e)))
        }
    }
}

/// Retrieves a job by its ID from the database
///
/// # Arguments
/// * `id` - UUID of the job to retrieve
/// * `config` - Shared configuration
///
/// # Returns
/// * `Result<JobItem, JobError>` - The job if found, or JobNotFound error
async fn get_job(id: Uuid, config: Arc<Config>) -> Result<JobItem, JobError> {
    let job = config.database().get_job_by_id(id).await.map_err(|e| JobError::Other(OtherError(e)))?;
    match job {
        Some(job) => Ok(job),
        None => Err(JobError::JobNotFound { id }),
    }
}

/// Increments a numeric value in the job metadata
///
/// # Arguments
/// * `metadata` - Current metadata map
/// * `key` - Key to increment
///
/// # Returns
/// * `Result<HashMap<String, String>, JobError>` - Updated metadata or an error
///
/// # Errors
/// * Returns KeyOutOfBounds if incrementing would exceed u64::MAX
/// * Returns error if value cannot be parsed as u64
pub fn increment_key_in_metadata(
    metadata: &HashMap<String, String>,
    key: &str,
) -> Result<HashMap<String, String>, JobError> {
    let mut new_metadata = metadata.clone();
    let attempt = get_u64_from_metadata(metadata, key).map_err(|e| JobError::Other(OtherError(e)))?;
    let incremented_value = attempt.checked_add(1);
    incremented_value.ok_or_else(|| JobError::KeyOutOfBounds { key: key.to_string() })?;
    new_metadata.insert(
        key.to_string(),
        incremented_value.ok_or(JobError::Other(OtherError(eyre!("Overflow while incrementing attempt"))))?.to_string(),
    );
    Ok(new_metadata)
}

/// Retrieves a u64 value from the metadata map
///
/// # Arguments
/// * `metadata` - Metadata map to search
/// * `key` - Key to retrieve
///
/// # Returns
/// * `color_eyre::Result<u64>` - The parsed value or an error
///
/// # Notes
/// * Returns 0 if the key doesn't exist in the metadata
/// * Wraps parsing errors with additional context
pub fn get_u64_from_metadata(metadata: &HashMap<String, String>, key: &str) -> color_eyre::Result<u64> {
    metadata
        .get(key)
        .unwrap_or(&"0".to_string())
        .parse::<u64>()
        .wrap_err(format!("Failed to parse u64 from metadata key '{}'", key))
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
    let job = get_job(id, config.clone()).await?;

    // Add to process queue directly
    add_job_to_process_queue(id, &job.job_type, config).await.map_err(|e| {
        tracing::error!(job_id = ?id, error = ?e, "Failed to add job to process queue");
        JobError::Other(OtherError(e))
    })?;

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
    let mut job = get_job(id, config.clone()).await?;
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
        .await
        .map_err(|e| JobError::Other(OtherError(e)))?;

    // Add to verification queue with appropriate delay
    add_job_to_verification_queue(
        id,
        &job.job_type,
        Duration::from_secs(job_handler.verification_polling_delay_seconds()),
        config,
    )
    .await
    .map_err(|e| {
        tracing::error!(
            job_id = ?id,
            error = ?e,
            "Failed to add job to verification queue"
        );
        JobError::Other(OtherError(e))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests for increment_key_in_metadata function
    mod test_increment_key_in_metadata {
        use super::*;

        #[test]
        /// Tests incrementing a non-existent key (should start at 0)
        fn key_does_not_exist() {
            let metadata = HashMap::new();
            let key = "test_key";
            let updated_metadata = increment_key_in_metadata(&metadata, key).unwrap();
            assert_eq!(updated_metadata.get(key), Some(&"1".to_string()));
        }

        #[test]
        /// Tests incrementing an existing numeric value
        fn key_exists_with_numeric_value() {
            let mut metadata = HashMap::new();
            metadata.insert("test_key".to_string(), "41".to_string());
            let key = "test_key";
            let updated_metadata = increment_key_in_metadata(&metadata, key).unwrap();
            assert_eq!(updated_metadata.get(key), Some(&"42".to_string()));
        }

        #[test]
        /// Tests handling of non-numeric values
        fn key_exists_with_non_numeric_value() {
            let mut metadata = HashMap::new();
            metadata.insert("test_key".to_string(), "not_a_number".to_string());
            let key = "test_key";
            let result = increment_key_in_metadata(&metadata, key);
            assert!(result.is_err());
        }

        #[test]
        /// Tests overflow handling at u64::MAX
        fn key_exists_with_max_u64_value() {
            let mut metadata = HashMap::new();
            metadata.insert("test_key".to_string(), u64::MAX.to_string());
            let key = "test_key";
            let result = increment_key_in_metadata(&metadata, key);
            assert!(result.is_err());
        }
    }

    /// Tests for get_u64_from_metadata function
    mod test_get_u64_from_metadata {
        use super::*;

        #[test]
        /// Tests retrieving a valid u64 value
        fn key_exists_with_valid_u64_value() {
            let mut metadata = HashMap::new();
            metadata.insert("key1".to_string(), "12345".to_string());
            let result = get_u64_from_metadata(&metadata, "key1").unwrap();
            assert_eq!(result, 12345);
        }

        #[test]
        /// Tests handling of invalid numeric strings
        fn key_exists_with_invalid_value() {
            let mut metadata = HashMap::new();
            metadata.insert("key2".to_string(), "not_a_number".to_string());
            let result = get_u64_from_metadata(&metadata, "key2");
            assert!(result.is_err());
        }

        #[test]
        /// Tests default behavior when key doesn't exist
        fn key_does_not_exist() {
            let metadata = HashMap::<String, String>::new();
            let result = get_u64_from_metadata(&metadata, "key3").unwrap();
            assert_eq!(result, 0);
        }
    }
}
