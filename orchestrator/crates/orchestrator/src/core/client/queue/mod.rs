pub mod error;
pub mod sqs;

use crate::types::queue::QueueType;
use async_trait::async_trait;
pub use error::QueueError;
use omniqueue::backends::{SqsConsumer, SqsProducer};
use omniqueue::Delivery;
use std::collections::HashMap;
use std::time::Duration;

/// Message with metadata
#[derive(Debug, Clone)]
pub struct Message<T> {
    /// The message payload
    pub payload: T,

    /// Message ID
    pub id: String,

    /// Receipt handle (for acknowledgment)
    pub receipt_handle: Option<String>,

    /// Message attributes
    pub attributes: HashMap<String, String>,
}

/// Trait defining queue operations
#[async_trait]
pub trait QueueClient: Send + Sync {
    async fn get_producer(&self, queue: QueueType) -> Result<SqsProducer, QueueError>;
    async fn get_consumer(&self, queue: QueueType) -> Result<SqsConsumer, QueueError>;
    async fn send_message(&self, queue: QueueType, payload: String, delay: Option<Duration>) -> Result<(), QueueError>;
    async fn consume_message_from_queue(&self, queue: QueueType) -> Result<Delivery, QueueError>;
}
