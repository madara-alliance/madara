use super::client::{
    alert::AlertError, cron::error::CronError, database::DatabaseError, queue::QueueError, storage::StorageError,
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

    #[error("Cron region: {0}")]
    CronError(#[from] CronError),

    #[error("Invalid provider: {0}")]
    InvalidProvider(String),
}
