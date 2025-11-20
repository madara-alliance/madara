pub mod error;
pub mod sqs;

use crate::types::queue::QueueType;
use async_trait::async_trait;
pub use error::QueueError;
use omniqueue::backends::{SqsConsumer, SqsProducer};
use omniqueue::Delivery;
use std::time::Duration;

/// Trait defining queue operations
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait QueueClient: Send + Sync {
    async fn get_producer(&self, queue: QueueType) -> Result<SqsProducer, QueueError>;
    async fn get_consumer(&self, queue: QueueType) -> Result<SqsConsumer, QueueError>;
    async fn send_message(&self, queue: QueueType, payload: String, delay: Option<Duration>) -> Result<(), QueueError>;
    async fn consume_message_from_queue(&self, queue: QueueType) -> Result<Delivery, QueueError>;

    /// Perform a health check on the queue service
    ///
    /// This method verifies that the queue service (e.g., AWS SQS) is accessible
    /// and the necessary permissions are in place.
    ///
    /// # Returns
    /// * `Ok(())` - If the queue service is healthy and accessible
    /// * `Err(QueueError)` - If the health check fails
    async fn health_check(&self) -> Result<(), QueueError>;
}
