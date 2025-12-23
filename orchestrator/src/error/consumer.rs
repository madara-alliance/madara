use crate::error::other::OtherError;
use crate::types::jobs::WorkerTriggerType;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug, PartialEq)]
pub enum ConsumptionError {
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

    #[error("Failed to parse message from queue, error {0}")]
    FailedToAcknowledgeMessage(String),

    #[error("Failed to nack message is queue, error {0}")]
    FailedToNackMessage(String),

    #[error("Queue Type not found: {0}")]
    QueueNotFound(String),
}
