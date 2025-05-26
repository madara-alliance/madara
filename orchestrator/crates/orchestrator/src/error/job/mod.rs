pub mod da_error;
pub mod fact;
pub mod proving;
pub mod snos;
pub mod state_update;

use crate::core::client::database::DatabaseError;
use crate::core::client::queue::QueueError;
use crate::core::client::storage::StorageError;
use crate::error::job::fact::FactError;
use crate::error::job::snos::SnosError;
use crate::error::other::OtherError;
use crate::error::ConsumptionError;
use crate::types::error::TypeError;
use crate::types::jobs::types::{JobStatus, JobType};
use da_error::DaError;
use orchestrator_prover_client_interface::ProverClientError;
use proving::ProvingError;
use state_update::StateUpdateError;
use thiserror::Error;
use uuid::Uuid;

pub type JobResult<T> = Result<T, JobError>;

/// Error types for job-related operations in the orchestrator
#[derive(Error, Debug)]
pub enum JobError {
    #[error("Failed to create job: {0}")]
    TypeError(#[from] TypeError),

    #[error("Failed to parse int: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),

    #[error("Failed to serialize data: {0}")]
    FailedToSerializeData(#[from] serde_json::Error),

    #[error("Queue error: {0}")]
    QueueError(#[from] QueueError),

    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),

    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    /// Wraps errors from fact operations
    #[error("Fact Error: {0}")]
    FactError(#[from] FactError),

    /// Wraps errors from SNOS operations
    #[error("Snos Error: {0}")]
    SnosJobError(#[from] SnosError),

    #[error("Provider Error: {0}")]
    ProviderError(String),

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

    /// Wraps errors from queue handling operations
    #[error("Queue Handling Error: {0}")]
    ConsumptionError(#[from] ConsumptionError),

    /// Wraps general errors that don't fit other categories
    #[error("Other error: {0}")]
    Other(#[from] OtherError),

    /// Indicates that the maximum capacity of jobs currently being processed has been reached
    #[error("Max Capacity Reached, Already processing")]
    MaxCapacityReached,

    /// Indicates an error occurred while extracting the processing lock
    #[error("Error extracting processing lock: {0}")]
    LockError(String),

    /// Indicates an error occurred while submitting a task to the prover client
    #[error("Error submitting task to prover client: {0}")]
    ProverClientError(#[from] ProverClientError),
}
