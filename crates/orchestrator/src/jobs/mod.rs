use crate::config::{config, Config};
use crate::jobs::constants::{JOB_PROCESS_ATTEMPT_METADATA_KEY, JOB_VERIFICATION_ATTEMPT_METADATA_KEY};
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::queue::job_queue::{add_job_to_process_queue, add_job_to_verification_queue};
use async_trait::async_trait;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use std::collections::HashMap;
use std::time::Duration;
use tracing::log;
use uuid::Uuid;

mod constants;
pub mod da_job;
pub mod types;

/// The Job trait is used to define the methods that a job
/// should implement to be used as a job for the orchestrator. The orchestrator automatically
/// handles queueing and processing of jobs as long as they implement the trait.
#[async_trait]
pub trait Job: Send + Sync {
    /// Should build a new job item and return it
    async fn create_job(&self, config: &dyn Config, internal_id: String) -> Result<JobItem>;
    /// Should process the job and return the external_id which can be used to
    /// track the status of the job. For example, a DA job will submit the state diff
    /// to the DA layer and return the txn hash.
    async fn process_job(&self, config: &dyn Config, job: &JobItem) -> Result<String>;
    /// Should verify the job and return the status of the verification. For example,
    /// a DA job will verify the inclusion of the state diff in the DA layer and return
    /// the status of the verification.
    async fn verify_job(&self, config: &dyn Config, job: &JobItem) -> Result<JobVerificationStatus>;
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
pub async fn create_job(job_type: JobType, internal_id: String) -> Result<()> {
    let config = config().await;
    let existing_job = config.database().get_job_by_internal_id_and_type(internal_id.as_str(), &job_type).await?;
    if existing_job.is_some() {
        log::debug!("Job already exists for internal_id {:?} and job_type {:?}. Skipping.", internal_id, job_type);
        return Err(eyre!(
            "Job already exists for internal_id {:?} and job_type {:?}. Skipping.",
            internal_id,
            job_type
        ));
    }

    let job_handler = get_job_handler(&job_type);
    let job_item = job_handler.create_job(config, internal_id).await?;
    config.database().create_job(job_item.clone()).await?;

    add_job_to_process_queue(job_item.id).await?;
    Ok(())
}

/// Processes the job, increments the process attempt count and updates the status of the job in the DB.
/// It then adds the job to the verification queue.
pub async fn process_job(id: Uuid) -> Result<()> {
    let config = config().await;
    let job = get_job(id).await?;

    match job.status {
        // we only want to process jobs that are in the created or verification failed state.
        // verification failed state means that the previous processing failed and we want to retry
        JobStatus::Created | JobStatus::VerificationFailed => {
            log::info!("Processing job with id {:?}", id);
        }
        _ => {
            log::error!("Invalid status {:?} for job with id {:?}. Cannot process.", id, job.status);
            return Err(eyre!("Invalid status {:?} for job with id {:?}. Cannot process.", id, job.status));
        }
    }
    // this updates the version of the job. this ensures that if another thread was about to process
    // the same job, it would fail to update the job in the database because the version would be outdated
    config.database().update_job_status(&job, JobStatus::LockedForProcessing).await?;

    let job_handler = get_job_handler(&job.job_type);
    let external_id = job_handler.process_job(config, &job).await?;

    let metadata = increment_key_in_metadata(&job.metadata, JOB_PROCESS_ATTEMPT_METADATA_KEY)?;
    config
        .database()
        .update_external_id_and_status_and_metadata(&job, external_id, JobStatus::PendingVerification, metadata)
        .await?;

    add_job_to_verification_queue(job.id, Duration::from_secs(job_handler.verification_polling_delay_seconds()))
        .await?;

    Ok(())
}

/// Verifies the job and updates the status of the job in the DB. If the verification fails, it retries
/// processing the job if the max attempts have not been exceeded. If the max attempts have been exceeded,
/// it marks the job as timedout. If the verification is still pending, it pushes the job back to the queue.
pub async fn verify_job(id: Uuid) -> Result<()> {
    let config = config().await;
    let job = get_job(id).await?;

    match job.status {
        JobStatus::PendingVerification => {
            log::info!("Verifying job with id {:?}", id);
        }
        _ => {
            log::error!("Invalid status {:?} for job with id {:?}. Cannot verify.", id, job.status);
            return Err(eyre!("Invalid status {:?} for job with id {:?}. Cannot verify.", id, job.status));
        }
    }

    let job_handler = get_job_handler(&job.job_type);
    let verification_status = job_handler.verify_job(config, &job).await?;

    match verification_status {
        JobVerificationStatus::Verified => {
            config.database().update_job_status(&job, JobStatus::Completed).await?;
        }
        JobVerificationStatus::Rejected => {
            config.database().update_job_status(&job, JobStatus::VerificationFailed).await?;

            // retry job processing if we haven't exceeded the max limit
            let process_attempts = get_u64_from_metadata(&job.metadata, JOB_PROCESS_ATTEMPT_METADATA_KEY)?;
            if process_attempts < job_handler.max_process_attempts() {
                log::info!(
                    "Verification failed for job {}. Retrying processing attempt {}.",
                    job.id,
                    process_attempts + 1
                );
                add_job_to_process_queue(job.id).await?;
                return Ok(());
            } else {
                // TODO: send alert
            }
        }
        JobVerificationStatus::Pending => {
            log::info!("Inclusion is still pending for job {}. Pushing back to queue.", job.id);
            let verify_attempts = get_u64_from_metadata(&job.metadata, JOB_VERIFICATION_ATTEMPT_METADATA_KEY)?;
            if verify_attempts >= job_handler.max_verification_attempts() {
                // TODO: send alert
                log::info!("Verification attempts exceeded for job {}. Marking as timedout.", job.id);
                config.database().update_job_status(&job, JobStatus::VerificationTimeout).await?;
                return Ok(());
            }
            let metadata = increment_key_in_metadata(&job.metadata, JOB_VERIFICATION_ATTEMPT_METADATA_KEY)?;
            config.database().update_metadata(&job, metadata).await?;
            add_job_to_verification_queue(
                job.id,
                Duration::from_secs(job_handler.verification_polling_delay_seconds()),
            )
            .await?;
        }
    };

    Ok(())
}

fn get_job_handler(job_type: &JobType) -> Box<dyn Job> {
    match job_type {
        JobType::DataSubmission => Box::new(da_job::DaJob),
        _ => unimplemented!("Job type not implemented yet."),
    }
}

async fn get_job(id: Uuid) -> Result<JobItem> {
    let config = config().await;
    let job = config.database().get_job_by_id(id).await?;
    match job {
        Some(job) => Ok(job),
        None => {
            log::error!("Failed to find job with id {:?}", id);
            Err(eyre!("Failed to process job with id {:?}", id))
        }
    }
}

fn increment_key_in_metadata(metadata: &HashMap<String, String>, key: &str) -> Result<HashMap<String, String>> {
    let mut new_metadata = metadata.clone();
    let attempt = get_u64_from_metadata(metadata, key)?;
    let incremented_value = attempt.checked_add(1);
    if incremented_value.is_none() {
        return Err(eyre!("Incrementing key {} in metadata would exceed u64::MAX", key));
    }
    new_metadata.insert(key.to_string(), incremented_value.unwrap().to_string());
    Ok(new_metadata)
}

fn get_u64_from_metadata(metadata: &HashMap<String, String>, key: &str) -> Result<u64> {
    Ok(metadata.get(key).unwrap_or(&"0".to_string()).parse::<u64>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod test_incremement_key_in_metadata {
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