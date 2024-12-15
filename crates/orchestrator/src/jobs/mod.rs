use std::collections::HashMap;
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use color_eyre::eyre::{eyre, Context};
use constants::{JOB_METADATA_ERROR, JOB_METADATA_FAILURE_REASON, JOB_METADATA_PROCESSING_COMPLETED_AT};
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
use types::JobItemUpdates;
use uuid::Uuid;

use crate::config::Config;
use crate::jobs::constants::{JOB_PROCESS_ATTEMPT_METADATA_KEY, JOB_VERIFICATION_ATTEMPT_METADATA_KEY};
#[double]
use crate::jobs::job_handler_factory::factory;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::queue::job_queue::{add_job_to_process_queue, add_job_to_verification_queue, ConsumptionError};

pub mod constants;
pub mod conversion;
pub mod da_job;
pub mod job_handler_factory;
pub mod proving_job;
pub mod register_proof_job;
pub mod snos_job;
pub mod state_update_job;
pub mod types;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum JobError {
    #[error("Job id {id:?} is invalid.")]
    InvalidId { id: String },

    #[error("Job already exists for internal_id {internal_id:?} and job_type {job_type:?}. Skipping!")]
    JobAlreadyExists { internal_id: String, job_type: JobType },

    #[error("Invalid status {job_status:?} for job with id {id:?}. Cannot process.")]
    InvalidStatus { id: Uuid, job_status: JobStatus },

    #[error("Failed to find job with id {id:?}")]
    JobNotFound { id: Uuid },

    #[error("Incrementing key {} in metadata would exceed u64::MAX", key)]
    KeyOutOfBounds { key: String },

    #[error("DA Error: {0}")]
    DaJobError(#[from] DaError),

    #[error("Proving Error: {0}")]
    ProvingJobError(#[from] ProvingError),

    #[error("Proving Error: {0}")]
    StateUpdateJobError(#[from] StateUpdateError),

    #[error("Snos Error: {0}")]
    SnosJobError(#[from] SnosError),

    #[error("Queue Handling Error: {0}")]
    ConsumptionError(#[from] ConsumptionError),

    #[error("Fact Error: {0}")]
    FactError(#[from] FactError),

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

// ====================================================
/// Wrapper Type for Other(<>) job type
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
// ====================================================

/// Job Trait
///
/// The Job trait is used to define the methods that a job
/// should implement to be used as a job for the orchestrator. The orchestrator automatically
/// handles queueing and processing of jobs as long as they implement the trait.
#[automock]
#[async_trait]
pub trait Job: Send + Sync {
    /// Should build a new job item and return it
    async fn create_job(
        &self,
        config: Arc<Config>,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError>;
    /// Should process the job and return the external_id which can be used to
    /// track the status of the job. For example, a DA job will submit the state diff
    /// to the DA layer and return the txn hash.
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError>;
    /// Should verify the job and return the status of the verification. For example,
    /// a DA job will verify the inclusion of the state diff in the DA layer and return
    /// the status of the verification.
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError>;
    /// Should return the maximum number of attempts to process the job. A new attempt is made
    /// every time the verification returns `JobVerificationStatus::Rejected`
    fn max_process_attempts(&self) -> u64;
    /// Should return the maximum number of attempts to verify the job. A new attempt is made
    /// every few seconds depending on the result `verification_polling_delay_seconds`
    fn max_verification_attempts(&self) -> u64;
    /// Should return the number of seconds to wait before polling for verification
    fn verification_polling_delay_seconds(&self) -> u64;
}

/// Creates the job in the DB in the created state and adds it to the process queue
#[tracing::instrument(fields(category = "general"), skip(config), ret, err)]
pub async fn create_job(
    job_type: JobType,
    internal_id: String,
    metadata: HashMap<String, String>,
    config: Arc<Config>,
) -> Result<(), JobError> {
    let start = Instant::now();
    tracing::info!(log_type = "starting", category = "general", function_type = "create_job", job_type = ?job_type, block_no = %internal_id, "General create job started for block");

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

    // this is technically a redundant check, we've another check inside `create_job`
    if existing_job.is_some() {
        tracing::warn!("{}", JobError::JobAlreadyExists { internal_id, job_type });
        return Ok(());
    }

    let job_handler = factory::get_job_handler(&job_type).await;
    let job_item = job_handler.create_job(config.clone(), internal_id.clone(), metadata).await?;
    config.database().create_job(job_item.clone()).await?;

    add_job_to_process_queue(job_item.id, &job_type, config.clone())
        .await
        .map_err(|e| JobError::Other(OtherError(e)))?;

    let attributes =
        [KeyValue::new("operation_job_type", format!("{:?}", job_type)), KeyValue::new("operation_type", "create_job")];

    tracing::info!(log_type = "completed", category = "general", function_type = "create_job", block_no = %internal_id, "General create job completed for block");
    let duration = start.elapsed();
    ORCHESTRATOR_METRICS.block_gauge.record(parse_string(&internal_id)?, &attributes);
    ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
    ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
    Ok(())
}

/// Processes the job, increments the process attempt count and updates the status of the job in the
/// DB. It then adds the job to the verification queue.
#[tracing::instrument(skip(config), fields(category = "general", job, job_type, internal_id), ret, err)]
pub async fn process_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let start = Instant::now();
    let job = get_job(id, config.clone()).await?;
    let internal_id = job.internal_id.clone();
    tracing::info!(log_type = "starting", category = "general", function_type = "process_job", block_no = %internal_id, "General process job started for block");

    tracing::Span::current().record("job", format!("{:?}", job.clone()));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type));
    tracing::Span::current().record("internal_id", job.internal_id.clone());

    tracing::debug!(job_id = ?id, status = ?job.status, "Current job status");
    match job.status {
        // we only want to process jobs that are in the created or verification failed state.
        // verification failed state means that the previous processing failed and we want to retry
        JobStatus::Created | JobStatus::VerificationFailed => {
            tracing::info!(job_id = ?id, status = ?job.status, "Job status is Created or VerificationFailed, proceeding with processing");
        }
        _ => {
            tracing::warn!(job_id = ?id, status = ?job.status, "Job status is Invalid. Cannot process.");
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }
    }
    // this updates the version of the job. this ensures that if another thread was about to process
    // the same job, it would fail to update the job in the database because the version would be
    // outdated
    tracing::debug!(job_id = ?id, "Updating job status to LockedForProcessing");
    let mut job = config
        .database()
        .update_job(&job, JobItemUpdates::new().update_status(JobStatus::LockedForProcessing).build())
        .await
        .map_err(|e| {
            tracing::error!(job_id = ?id, error = ?e, "Failed to update job status");
            JobError::Other(OtherError(e))
        })?;

    tracing::debug!(job_id = ?id, job_type = ?job.job_type, "Getting job handler");
    let job_handler = factory::get_job_handler(&job.job_type).await;
    let external_id = match AssertUnwindSafe(job_handler.process_job(config.clone(), &mut job)).catch_unwind().await {
        Ok(Ok(external_id)) => {
            tracing::debug!(job_id = ?id, "Successfully processed job");
            // Add the time of processing to the metadata.
            job.metadata
                .insert(JOB_METADATA_PROCESSING_COMPLETED_AT.to_string(), Utc::now().timestamp_millis().to_string());
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
    tracing::debug!(job_id = ?id, "Incrementing process attempt count in metadata");
    let metadata = increment_key_in_metadata(&job.metadata, JOB_PROCESS_ATTEMPT_METADATA_KEY)?;

    // Fetching the job again because update status above will update the job version
    tracing::debug!(job_id = ?id, "Updating job status to PendingVerification");
    config
        .database()
        .update_job(
            &job,
            JobItemUpdates::new()
                .update_status(JobStatus::PendingVerification)
                .update_metadata(metadata)
                .update_external_id(external_id.into())
                .build(),
        )
        .await
        .map_err(|e| {
            tracing::error!(job_id = ?id, error = ?e, "Failed to update job status");
            JobError::Other(OtherError(e))
        })?;

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

    let attributes = [
        KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        KeyValue::new("operation_type", "process_job"),
    ];

    tracing::info!(log_type = "completed", category = "general", function_type = "process_job", block_no = %internal_id, "General process job completed for block");
    let duration = start.elapsed();
    ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
    ORCHESTRATOR_METRICS.block_gauge.record(parse_string(&job.internal_id)?, &attributes);
    ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
    Ok(())
}

/// Verify Job Function
///
/// Verifies the job and updates the status of the job in the DB. If the verification fails, it
/// retries processing the job if the max attempts have not been exceeded. If the max attempts have
/// been exceeded, it marks the job as timed out. If the verification is still pending, it pushes
/// the job back to the queue.
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
        JobStatus::PendingVerification => {
            tracing::debug!(job_id = ?id, "Job status is PendingVerification, proceeding with verification");
        }
        _ => {
            tracing::error!(job_id = ?id, status = ?job.status, "Invalid job status for verification");
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }
    }

    let job_handler = factory::get_job_handler(&job.job_type).await;
    tracing::debug!(job_id = ?id, "Verifying job with handler");
    let verification_status = job_handler.verify_job(config.clone(), &mut job).await?;
    tracing::Span::current().record("verification_status", format!("{:?}", &verification_status));

    let attributes = [
        KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        KeyValue::new("operation_type", "verify_job"),
        KeyValue::new("operation_verification_status", format!("{:?}", &verification_status)),
    ];

    match verification_status {
        JobVerificationStatus::Verified => {
            tracing::info!(job_id = ?id, "Job verified successfully");
            match job
                .metadata
                .get(JOB_METADATA_PROCESSING_COMPLETED_AT)
                .and_then(|time| time.parse::<i64>().ok())
                .map(|start| Utc::now().timestamp_millis() - start)
            {
                Some(time_taken) => ORCHESTRATOR_METRICS
                    .verification_time
                    .record(time_taken as f64, &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))]),
                None => tracing::warn!("Failed to calculate verification time: Invalid or missing processing time"),
            }
            let mut metadata = job.metadata.clone();
            metadata.remove("processing_completed_at");
            config
                .database()
                .update_job(
                    &job,
                    JobItemUpdates::new().update_metadata(metadata).update_status(JobStatus::Completed).build(),
                )
                .await
                .map_err(|e| {
                    tracing::error!(job_id = ?id, error = ?e, "Failed to update job status to Completed");
                    JobError::Other(OtherError(e))
                })?;
        }
        JobVerificationStatus::Rejected(e) => {
            tracing::warn!(job_id = ?id, error = ?e, "Job verification rejected");
            let mut new_job = job.clone();
            new_job.metadata.insert(JOB_METADATA_ERROR.to_string(), e);
            new_job.status = JobStatus::VerificationFailed;

            let process_attempts = get_u64_from_metadata(&job.metadata, JOB_PROCESS_ATTEMPT_METADATA_KEY)
                .map_err(|e| JobError::Other(OtherError(e)))?;
            if process_attempts < job_handler.max_process_attempts() {
                tracing::info!(
                    job_id = ?id,
                    attempt = process_attempts + 1,
                    "Verification failed. Retrying job processing"
                );

                config
                    .database()
                    .update_job(
                        &job,
                        JobItemUpdates::new()
                            .update_status(JobStatus::VerificationFailed)
                            .update_metadata(new_job.metadata)
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
                    format!("Verification rejected. Max process attempts reached: {}", process_attempts),
                )
                .await;
            }
        }
        JobVerificationStatus::Pending => {
            tracing::debug!(job_id = ?id, "Job verification still pending");
            let verify_attempts = get_u64_from_metadata(&job.metadata, JOB_VERIFICATION_ATTEMPT_METADATA_KEY)
                .map_err(|e| JobError::Other(OtherError(e)))?;
            if verify_attempts >= job_handler.max_verification_attempts() {
                tracing::warn!(job_id = ?id, "Max verification attempts reached. Marking job as timed out");
                config
                    .database()
                    .update_job(&job, JobItemUpdates::new().update_status(JobStatus::VerificationTimeout).build())
                    .await
                    .map_err(|e| {
                        tracing::error!(job_id = ?id, error = ?e, "Failed to update job status to VerificationTimeout");
                        JobError::Other(OtherError(e))
                    })?;
                return Ok(());
            }
            let metadata = increment_key_in_metadata(&job.metadata, JOB_VERIFICATION_ATTEMPT_METADATA_KEY)?;

            config.database().update_job(&job, JobItemUpdates::new().update_metadata(metadata).build()).await.map_err(
                |e| {
                    tracing::error!(job_id = ?id, error = ?e, "Failed to update job metadata");
                    JobError::Other(OtherError(e))
                },
            )?;

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
    };

    tracing::info!(log_type = "completed", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job completed for block");
    let duration = start.elapsed();
    ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
    ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
    ORCHESTRATOR_METRICS.block_gauge.record(parse_string(&job.internal_id)?, &attributes);
    Ok(())
}

/// Terminates the job and updates the status of the job in the DB.
/// Logs error if the job status `Completed` is existing on DL queue.
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

    let mut metadata = job.metadata.clone();
    let internal_id = job.internal_id.clone();
    metadata.insert(JOB_METADATA_FAILURE_REASON.to_string(), reason);

    tracing::debug!(job_id = ?job.id, "Updating job status to Failed in database");
    match config
        .database()
        .update_job(job, JobItemUpdates::new().update_status(JobStatus::Failed).update_metadata(metadata).build())
        .await
    {
        Ok(_) => {
            tracing::info!(log_type = "completed", category = "general", function_type = "handle_job_failure", block_no = %internal_id, "General handle job failure completed for block");
            ORCHESTRATOR_METRICS
                .failed_jobs
                .add(1.0, &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))]);
            Ok(())
        }
        Err(e) => {
            tracing::error!(log_type = "error", category = "general", function_type = "handle_job_failure",  block_no = %internal_id, error = %e, "General handle job failure failed for block");
            Err(JobError::Other(OtherError(e)))
        }
    }
}

async fn get_job(id: Uuid, config: Arc<Config>) -> Result<JobItem, JobError> {
    let job = config.database().get_job_by_id(id).await.map_err(|e| JobError::Other(OtherError(e)))?;
    match job {
        Some(job) => Ok(job),
        None => Err(JobError::JobNotFound { id }),
    }
}

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

fn get_u64_from_metadata(metadata: &HashMap<String, String>, key: &str) -> color_eyre::Result<u64> {
    metadata
        .get(key)
        .unwrap_or(&"0".to_string())
        .parse::<u64>()
        .wrap_err(format!("Failed to parse u64 from metadata key '{}'", key))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod test_increment_key_in_metadata {
        use super::*;

        #[test]
        fn key_does_not_exist() {
            let metadata = HashMap::new();
            let key = "test_key";
            let updated_metadata = increment_key_in_metadata(&metadata, key).unwrap();
            assert_eq!(updated_metadata.get(key), Some(&"1".to_string()));
        }

        #[test]
        fn key_exists_with_numeric_value() {
            let mut metadata = HashMap::new();
            metadata.insert("test_key".to_string(), "41".to_string());
            let key = "test_key";
            let updated_metadata = increment_key_in_metadata(&metadata, key).unwrap();
            assert_eq!(updated_metadata.get(key), Some(&"42".to_string()));
        }

        #[test]
        fn key_exists_with_non_numeric_value() {
            let mut metadata = HashMap::new();
            metadata.insert("test_key".to_string(), "not_a_number".to_string());
            let key = "test_key";
            let result = increment_key_in_metadata(&metadata, key);
            assert!(result.is_err());
        }

        #[test]
        fn key_exists_with_max_u64_value() {
            let mut metadata = HashMap::new();
            metadata.insert("test_key".to_string(), u64::MAX.to_string());
            let key = "test_key";
            let result = increment_key_in_metadata(&metadata, key);
            assert!(result.is_err());
        }
    }

    mod test_get_u64_from_metadata {
        use super::*;

        #[test]
        fn key_exists_with_valid_u64_value() {
            let mut metadata = HashMap::new();
            metadata.insert("key1".to_string(), "12345".to_string());
            let result = get_u64_from_metadata(&metadata, "key1").unwrap();
            assert_eq!(result, 12345);
        }

        #[test]
        fn key_exists_with_invalid_value() {
            let mut metadata = HashMap::new();
            metadata.insert("key2".to_string(), "not_a_number".to_string());
            let result = get_u64_from_metadata(&metadata, "key2");
            assert!(result.is_err());
        }

        #[test]
        fn key_does_not_exist() {
            let metadata = HashMap::<String, String>::new();
            let result = get_u64_from_metadata(&metadata, "key3").unwrap();
            assert_eq!(result, 0);
        }
    }
}
