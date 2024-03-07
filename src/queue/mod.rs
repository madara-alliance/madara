pub mod job_queue;
pub mod sqs;

use async_trait::async_trait;
use color_eyre::Result;
use omniqueue::{Delivery, QueueError};
use std::hash::Hash;
use std::time::Duration;

#[async_trait]
pub trait QueueProvider: Send + Sync {
    async fn send_message_to_queue(&self, queue: String, payload: String, delay: Option<Duration>) -> Result<()>;
    async fn consume_message_from_queue(&self, queue: String) -> std::result::Result<Delivery, QueueError>;
}

pub async fn init_consumers() -> Result<()> {
    Ok(job_queue::init_consumers().await?)
}
