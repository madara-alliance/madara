pub mod da;
pub mod proving;
pub mod snos;
pub mod state_update;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::status::JobVerificationStatus;
use crate::utils::helpers::JobProcessingState;
use async_trait::async_trait;
use std::sync::Arc;

/// Job Trait
///
/// The Job trait is used to define the methods that a job
/// should implement to be used as a job for the orchestrator. The orchestrator automatically
/// handles queueing and processing of jobs as long as they implement the trait.
///
/// # Implementation Requirements
/// Implementors must be both `Send` and `Sync` to work with the async processing system.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JobHandlerTrait: Send + Sync {
    /// Should build a new job item and return it
    ///
    /// # Arguments
    /// * `config` - Shared configuration for the job
    /// * `internal_id` - Unique identifier for internal tracking
    /// * `metadata` - Additional key-value pairs associated with the job
    ///
    /// # Returns
    /// * `Result<JobItem, JobError>` - The created job item or an error
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError>;

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
    fn max_process_attempts(&self) -> u64;

    /// Should return the maximum number of attempts to verify the job. A new attempt is made
    /// every few seconds depending on the result `verification_polling_delay_seconds`
    fn max_verification_attempts(&self) -> u64;

    /// Should return the number of seconds to wait before polling for verification
    fn verification_polling_delay_seconds(&self) -> u64;
    fn job_processing_lock(&self, config: Arc<Config>) -> Option<Arc<JobProcessingState>>;
}
