use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use color_eyre::eyre::{Context, eyre};
use da_job::DaError;
use mockall::automock;
use mockall_double::double;
use opentelemetry::KeyValue;
use proving_job::ProvingError;
use snos_job::SnosError;
use snos_job::error::FactError;
use state_update_job::StateUpdateError;
use tracing::log;
use uuid::Uuid;

use crate::config::Config;
use crate::jobs::constants::{JOB_PROCESS_ATTEMPT_METADATA_KEY, JOB_VERIFICATION_ATTEMPT_METADATA_KEY};
#[double]
use crate::jobs::job_handler_factory::factory;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::queue::job_queue::{ConsumptionError, add_job_to_process_queue, add_job_to_verification_queue};

pub mod constants;
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
    #[error("Job already exists for internal_id {internal_id:?} and job_type {job_type:?}. Skipping!")]
    JobAlreadyExists { internal_id: String, job_type: JobType },

    #[error("Invalid status {id:?} for job with id {job_status:?}. Cannot process.")]
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
#[tracing::instrument(fields(category = "general"), skip(config))]
pub async fn create_job(
    job_type: JobType,
    internal_id: String,
    metadata: HashMap<String, String>,
    config: Arc<Config>,
) -> Result<(), JobError> {
    let existing_job = config
        .database()
        .get_job_by_internal_id_and_type(internal_id.as_str(), &job_type)
        .await
        .map_err(|e| JobError::Other(OtherError(e)))?;

    if existing_job.is_some() {
        return Err(JobError::JobAlreadyExists { internal_id, job_type });
    }

    let job_handler = factory::get_job_handler(&job_type).await;
    let job_item = job_handler.create_job(config.clone(), internal_id.clone(), metadata).await?;
    config.database().create_job(job_item.clone()).await.map_err(|e| JobError::Other(OtherError(e)))?;

    add_job_to_process_queue(job_item.id, config.clone()).await.map_err(|e| JobError::Other(OtherError(e)))?;

    let attributes = [
        KeyValue::new("job_type", format!("{:?}", job_type)),
        KeyValue::new("type", "create_job"),
        KeyValue::new("job", format!("{:?}", job_item)),
    ];

    ORCHESTRATOR_METRICS.block_gauge.record(internal_id.parse::<f64>().unwrap(), &attributes);
    Ok(())
}

/// Processes the job, increments the process attempt count and updates the status of the job in the
/// DB. It then adds the job to the verification queue.
#[tracing::instrument(skip(config), fields(category = "general", job, job_type, internal_id))]
pub async fn process_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let mut job = get_job(id, config.clone()).await?;

    tracing::Span::current().record("job", format!("{:?}", job.clone()));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type.clone()));
    tracing::Span::current().record("internal_id", job.internal_id.clone());

    match job.status {
        // we only want to process jobs that are in the created or verification failed state.
        // verification failed state means that the previous processing failed and we want to retry
        JobStatus::Created | JobStatus::VerificationFailed => {
            log::info!("Processing job with id {:?}", id);
        }
        _ => {
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }
    }
    // this updates the version of the job. this ensures that if another thread was about to process
    // the same job, it would fail to update the job in the database because the version would be
    // outdated
    config
        .database()
        .update_job_status(&job, JobStatus::LockedForProcessing)
        .await
        .map_err(|e| JobError::Other(OtherError(e)))?;

    let job_handler = factory::get_job_handler(&job.job_type).await;
    let external_id = job_handler.process_job(config.clone(), &mut job).await?;
    let metadata = increment_key_in_metadata(&job.metadata, JOB_PROCESS_ATTEMPT_METADATA_KEY)?;

    // Fetching the job again because update status above will update the job version
    let mut job_updated = get_job(id, config.clone()).await?;

    job_updated.external_id = external_id.into();
    job_updated.status = JobStatus::PendingVerification;
    job_updated.metadata = metadata;

    config.database().update_job(&job_updated).await.map_err(|e| JobError::Other(OtherError(e)))?;

    add_job_to_verification_queue(
        job.id,
        Duration::from_secs(job_handler.verification_polling_delay_seconds()),
        config.clone(),
    )
    .await
    .map_err(|e| JobError::Other(OtherError(e)))?;

    let attributes = [
        KeyValue::new("job_type", format!("{:?}", job.job_type)),
        KeyValue::new("type", "process_job"),
        KeyValue::new("job", format!("{:?}", job)),
    ];

    ORCHESTRATOR_METRICS.block_gauge.record(job.internal_id.parse::<f64>().unwrap(), &attributes);

    Ok(())
}

/// Verify Job Function
///
/// Verifies the job and updates the status of the job in the DB. If the verification fails, it
/// retries processing the job if the max attempts have not been exceeded. If the max attempts have
/// been exceeded, it marks the job as timed out. If the verification is still pending, it pushes
/// the job back to the queue.
#[tracing::instrument(skip(config), fields(category = "general", job, job_type, internal_id, verification_status))]
pub async fn verify_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let mut job = get_job(id, config.clone()).await?;

    tracing::Span::current().record("job", format!("{:?}", job.clone()));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type.clone()));
    tracing::Span::current().record("internal_id", job.internal_id.clone());

    match job.status {
        JobStatus::PendingVerification => {
            log::info!("Verifying job with id {:?}", id);
        }
        _ => {
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }
    }

    let job_handler = factory::get_job_handler(&job.job_type).await;
    let verification_status = job_handler.verify_job(config.clone(), &mut job).await?;
    tracing::Span::current().record("verification_status", format!("{:?}", verification_status.clone()));

    match verification_status {
        JobVerificationStatus::Verified => {
            config
                .database()
                .update_job_status(&job, JobStatus::Completed)
                .await
                .map_err(|e| JobError::Other(OtherError(e)))?;
        }
        JobVerificationStatus::Rejected(e) => {
            let mut new_job = job.clone();
            new_job.metadata.insert("error".to_string(), e);
            new_job.status = JobStatus::VerificationFailed;

            config.database().update_job(&new_job).await.map_err(|e| JobError::Other(OtherError(e)))?;

            log::error!("Verification failed for job with id {:?}. Cannot verify.", id);

            // retry job processing if we haven't exceeded the max limit
            let process_attempts = get_u64_from_metadata(&job.metadata, JOB_PROCESS_ATTEMPT_METADATA_KEY)
                .map_err(|e| JobError::Other(OtherError(e)))?;
            if process_attempts < job_handler.max_process_attempts() {
                log::info!(
                    "Verification failed for job {}. Retrying processing attempt {}.",
                    job.id,
                    process_attempts + 1
                );
                add_job_to_process_queue(job.id, config.clone()).await.map_err(|e| JobError::Other(OtherError(e)))?;
                return Ok(());
            }
        }
        JobVerificationStatus::Pending => {
            log::info!("Inclusion is still pending for job {}. Pushing back to queue.", job.id);
            let verify_attempts = get_u64_from_metadata(&job.metadata, JOB_VERIFICATION_ATTEMPT_METADATA_KEY)
                .map_err(|e| JobError::Other(OtherError(e)))?;
            if verify_attempts >= job_handler.max_verification_attempts() {
                log::info!("Verification attempts exceeded for job {}. Marking as timed out.", job.id);
                config
                    .database()
                    .update_job_status(&job, JobStatus::VerificationTimeout)
                    .await
                    .map_err(|e| JobError::Other(OtherError(e)))?;
                return Ok(());
            }
            let metadata = increment_key_in_metadata(&job.metadata, JOB_VERIFICATION_ATTEMPT_METADATA_KEY)?;
            config.database().update_metadata(&job, metadata).await.map_err(|e| JobError::Other(OtherError(e)))?;
            add_job_to_verification_queue(
                job.id,
                Duration::from_secs(job_handler.verification_polling_delay_seconds()),
                config.clone(),
            )
            .await
            .map_err(|e| JobError::Other(OtherError(e)))?;
        }
    };

    let attributes = [
        KeyValue::new("job_type", format!("{:?}", job.job_type)),
        KeyValue::new("type", "verify_job"),
        KeyValue::new("job", format!("{:?}", job)),
    ];

    ORCHESTRATOR_METRICS.block_gauge.record(job.internal_id.parse::<f64>().unwrap(), &attributes);

    Ok(())
}

/// Terminates the job and updates the status of the job in the DB.
/// Logs error if the job status `Completed` is existing on DL queue.
#[tracing::instrument(skip(config), fields(job_status, job_type))]
pub async fn handle_job_failure(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
    let mut job = get_job(id, config.clone()).await?.clone();
    let mut metadata = job.metadata.clone();

    tracing::Span::current().record("job_status", format!("{:?}", job.status));
    tracing::Span::current().record("job_type", format!("{:?}", job.job_type));

    if job.status == JobStatus::Completed {
        log::error!("Invalid state exists on DL queue: {}", job.status.to_string());
        return Ok(());
    }
    // We assume that a Failure status wil only show up if the message is sent twice from a queue
    // Can return silently because it's already been processed.
    else if job.status == JobStatus::Failed {
        return Ok(());
    }

    metadata.insert("last_job_status".to_string(), job.status.to_string());
    job.metadata = metadata;
    job.status = JobStatus::Failed;

    config.database().update_job(&job).await.map_err(|e| JobError::Other(OtherError(e)))?;

    Ok(())
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
    new_metadata.insert(key.to_string(), incremented_value.unwrap().to_string());
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
