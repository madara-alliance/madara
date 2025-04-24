use crate::core::client::alert::AlertError;
use crate::core::client::database::DatabaseError;
use crate::core::client::queue::QueueError;
use crate::core::client::storage::StorageError;
use crate::core::error::OrchestratorCoreError;
use crate::error::other::OtherError;
use crate::error::ConsumptionError;
use crate::types::jobs::WorkerTriggerType;
use crate::types::queue::QueueType;
use crate::OrchestratorError;
use thiserror::Error;
use uuid::Uuid;

/// Result type for orchestrator operations
pub type EventSystemResult<T> = Result<T, EventSystemError>;

/// EventSystemError - Error type for event system
/// This error type is used to handle errors that occur during the event system
/// It is used to handle errors that occur during the event system
///
#[derive(Error, Debug)]
pub enum EventSystemError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    #[error("Alert error: {0}")]
    AlertError(#[from] AlertError),

    #[error("Queue error: {0}")]
    QueueCoreError(#[from] QueueError),

    #[error("Database error: {0}")]
    DatabaseCoreError(#[from] DatabaseError),

    #[error("Orchestrator Core Error: {0}")]
    OrchestratorCoreError(#[from] OrchestratorCoreError),

    #[error("Event Handler Already existing for Queue Type : {0:?}")]
    EventHandlerAlreadyExisting(QueueType),

    #[error("Failed to consume message from queue, error {error_msg:?}")]
    FailedToConsumeFromQueue { error_msg: String },

    #[error("Failed to handle job with id {job_id:?}. Error: {error_msg:?}")]
    FailedToHandleJob { job_id: Uuid, error_msg: String },

    #[error("Failed to spawn {worker_trigger_type:?} worker. Error: {error_msg:?}")]
    FailedToSpawnWorker { worker_trigger_type: WorkerTriggerType, error_msg: String },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),

    #[error("Message Parsing Serde Error: {0}")]
    PayloadSerdeError(String),

    #[error("OrchestratorError: {0}")]
    FromOrchestratorError(#[from] OrchestratorError),

    #[error("ConsumptionError: {0}")]
    FromConsumptionError(#[from] ConsumptionError),

    #[error("Invalid job type: {0}")]
    InvalidJobType(String),
}
