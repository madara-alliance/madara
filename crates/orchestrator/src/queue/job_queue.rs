use std::future::Future;
use std::time::Duration;

use color_eyre::eyre::eyre;
use color_eyre::Result;
use omniqueue::QueueError;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::log;
use uuid::Uuid;

use crate::config::config;
use crate::jobs::{handle_job_failure, process_job, verify_job};

pub const JOB_PROCESSING_QUEUE: &str = "madara_orchestrator_job_processing_queue";
pub const JOB_VERIFICATION_QUEUE: &str = "madara_orchestrator_job_verification_queue";
// Below is the Data Letter Queue for the the above two jobs.
pub const JOB_HANDLE_FAILURE_QUEUE: &str = "madara_orchestrator_job_handle_failure_queue";

#[derive(Debug, Serialize, Deserialize)]
pub struct JobQueueMessage {
    pub(crate) id: Uuid,
}

pub async fn add_job_to_process_queue(id: Uuid) -> Result<()> {
    log::info!("Adding job with id {:?} to processing queue", id);
    add_job_to_queue(id, JOB_PROCESSING_QUEUE.to_string(), None).await
}

pub async fn add_job_to_verification_queue(id: Uuid, delay: Duration) -> Result<()> {
    log::info!("Adding job with id {:?} to verification queue", id);
    add_job_to_queue(id, JOB_VERIFICATION_QUEUE.to_string(), Some(delay)).await
}

pub async fn consume_job_from_queue<F, Fut>(queue: String, handler: F) -> Result<()>
where
    F: FnOnce(Uuid) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    log::info!("Consuming from queue {:?}", queue);
    let config = config().await;
    let delivery = match config.queue().consume_message_from_queue(queue.clone()).await {
        Ok(d) => d,
        Err(QueueError::NoData) => {
            return Ok(());
        }
        Err(e) => {
            return Err(eyre!("Failed to consume message from queue, error {}", e));
        }
    };
    let job_message: Option<JobQueueMessage> = delivery.payload_serde_json()?;

    match job_message {
        Some(job_message) => {
            log::info!("Handling job with id {:?} for queue {:?}", job_message.id, queue);
            match handler(job_message.id).await {
                Ok(_) => delivery.ack().await.map_err(|(e, _)| e)?,
                Err(e) => {
                    log::error!("Failed to handle job with id {:?}. Error: {:?}", job_message.id, e);

                    // if the queue as a retry logic at the source, it will be attempted
                    // after the nack
                    delivery.nack().await.map_err(|(e, _)| e)?;
                }
            };
        }
        None => return Ok(()),
    };

    Ok(())
}

macro_rules! spawn_consumer {
    ($queue_type :expr, $handler : expr) => {
        tokio::spawn(async move {
            loop {
                match consume_job_from_queue($queue_type, $handler).await {
                    Ok(_) => {}
                    Err(e) => log::error!("Failed to consume from queue {:?}. Error: {:?}", $queue_type, e),
                }
                sleep(Duration::from_secs(1)).await;
            }
        });
    };
}

pub async fn init_consumers() -> Result<()> {
    spawn_consumer!(JOB_PROCESSING_QUEUE.to_string(), process_job);
    spawn_consumer!(JOB_VERIFICATION_QUEUE.to_string(), verify_job);
    spawn_consumer!(JOB_HANDLE_FAILURE_QUEUE.to_string(), handle_job_failure);

    Ok(())
}

async fn add_job_to_queue(id: Uuid, queue: String, delay: Option<Duration>) -> Result<()> {
    let config = config().await;
    let message = JobQueueMessage { id };
    config.queue().send_message_to_queue(queue, serde_json::to_string(&message)?, delay).await?;
    Ok(())
}
