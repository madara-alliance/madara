use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use color_eyre::Result;
use lazy_static::lazy_static;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::{Delivery, QueueError};
use utils::env_utils::get_env_var_or_panic;

use crate::queue::job_queue::{
    JOB_HANDLE_FAILURE_QUEUE, JOB_PROCESSING_QUEUE, JOB_VERIFICATION_QUEUE, WORKER_TRIGGER_QUEUE,
};
use crate::queue::QueueProvider;
pub struct SqsQueue;

lazy_static! {
    /// Maps Queue Name to Env var of queue URL.
    pub static ref QUEUE_NAME_TO_ENV_VAR_MAPPING: HashMap<&'static str, &'static str> = HashMap::from([
        (JOB_PROCESSING_QUEUE, "SQS_JOB_PROCESSING_QUEUE_URL"),
        (JOB_VERIFICATION_QUEUE, "SQS_JOB_VERIFICATION_QUEUE_URL"),
        (JOB_HANDLE_FAILURE_QUEUE, "SQS_JOB_HANDLE_FAILURE_QUEUE_URL"),
        (WORKER_TRIGGER_QUEUE, "SQS_WORKER_TRIGGER_QUEUE_URL"),
    ]);
}

#[async_trait]
impl QueueProvider for SqsQueue {
    async fn send_message_to_queue(&self, queue: String, payload: String, delay: Option<Duration>) -> Result<()> {
        let queue_url = get_queue_url(queue);
        let producer = get_producer(queue_url).await?;

        match delay {
            Some(d) => producer.send_raw_scheduled(payload.as_str(), d).await?,
            None => producer.send_raw(payload.as_str()).await?,
        }

        Ok(())
    }

    async fn consume_message_from_queue(&self, queue: String) -> std::result::Result<Delivery, QueueError> {
        let queue_url = get_queue_url(queue);
        let mut consumer = get_consumer(queue_url).await?;
        consumer.receive().await
    }
}

/// To fetch the queue URL from the environment variables
fn get_queue_url(queue_name: String) -> String {
    get_env_var_or_panic(
        QUEUE_NAME_TO_ENV_VAR_MAPPING.get(queue_name.as_str()).expect("Not able to get the queue env var name."),
    )
}

// TODO: store the producer and consumer in memory to avoid creating a new one every time
async fn get_producer(queue: String) -> Result<SqsProducer> {
    let (producer, _) =
        SqsBackend::builder(SqsConfig { queue_dsn: queue, override_endpoint: true }).build_pair().await?;
    Ok(producer)
}

async fn get_consumer(queue: String) -> std::result::Result<SqsConsumer, QueueError> {
    let (_, consumer) =
        SqsBackend::builder(SqsConfig { queue_dsn: queue, override_endpoint: true }).build_pair().await?;
    Ok(consumer)
}
