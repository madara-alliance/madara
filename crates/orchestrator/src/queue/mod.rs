pub mod job_queue;
pub mod sqs;

use std::time::Duration;

use async_trait::async_trait;
use color_eyre::Result as EyreResult;
use mockall::automock;
use omniqueue::{Delivery, QueueError};

use crate::jobs::JobError;

/// The QueueProvider trait is used to define the methods that a queue
/// should implement to be used as a queue for the orchestrator. The
/// purpose of this trait is to allow developers to use any queue of their choice.
#[automock]
#[async_trait]
pub trait QueueProvider: Send + Sync {
    async fn send_message_to_queue(&self, queue: String, payload: String, delay: Option<Duration>) -> EyreResult<()>;
    async fn consume_message_from_queue(&self, queue: String) -> std::result::Result<Delivery, QueueError>;
}

pub async fn init_consumers() -> Result<(), JobError> {
    job_queue::init_consumers().await
}
