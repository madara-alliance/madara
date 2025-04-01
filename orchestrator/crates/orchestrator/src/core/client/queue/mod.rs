pub mod sqs;

use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use crate::error::Result;

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
    /// Connect to the queue service
    // async fn connect(&self) -> Result<()>;
    //
    // /// Send a message to the queue
    // async fn send_message<T>(&self, message: &T) -> Result<String>
    // where
    //     T: Serialize + Send + Sync;
    //
    // /// Send a message to the queue with a delay
    // async fn send_message_with_delay<T>(&self, message: &T, delay: Duration) -> Result<String>
    // where
    //     T: Serialize + Send + Sync;
    //
    // /// Receive messages from the queue
    // async fn receive_messages<T>(&self, max_messages: i32, wait_time: Duration) -> Result<Vec<Message<T>>>
    // where
    //     T: DeserializeOwned + Send + Sync;
    //
    // /// Delete a message from the queue (acknowledge)
    // async fn delete_message(&self, receipt_handle: &str) -> Result<()>;
    //
    // /// Change the visibility timeout of a message
    // async fn change_message_visibility(&self, receipt_handle: &str, visibility_timeout: Duration) -> Result<()>;
    //
    // /// Purge all messages from the queue
    // async fn purge_queue(&self) -> Result<()>;
}
