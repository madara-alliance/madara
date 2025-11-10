use super::client::{
    alert::AlertError, database::DatabaseError, event_bus::error::EventBusError, lock::error::LockError,
    queue::QueueError, storage::StorageError,
};
use thiserror::Error;

pub type OrchestratorCoreResult<T> = Result<T, OrchestratorCoreError>;

#[derive(Error, Debug)]
pub enum OrchestratorCoreError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    #[error("Alert error: {0}")]
    AlertError(#[from] AlertError),

    #[error("Queue error: {0}")]
    QueueError(#[from] QueueError),

    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),

    #[error("EventBus Error: {0}")]
    CronError(#[from] EventBusError),

    #[error("Lock Error: {0}")]
    LockError(#[from] LockError),

    #[error("Invalid provider: {0}")]
    InvalidProvider(String),

    #[error("Invalid versioned constants file: {0}")]
    InvalidVersionedConstantsFile(#[from] blockifier::blockifier_versioned_constants::VersionedConstantsError),
}
