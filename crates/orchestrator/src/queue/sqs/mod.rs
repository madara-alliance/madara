use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use lazy_static::lazy_static;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::{Delivery, QueueError};
use utils::env_utils::get_env_var_or_panic;

use crate::queue::job_queue::{
    DATA_SUBMISSION_JOB_PROCESSING_QUEUE, DATA_SUBMISSION_JOB_VERIFICATION_QUEUE, JOB_HANDLE_FAILURE_QUEUE,
    PROOF_REGISTRATION_JOB_PROCESSING_QUEUE, PROOF_REGISTRATION_JOB_VERIFICATION_QUEUE, PROVING_JOB_PROCESSING_QUEUE,
    PROVING_JOB_VERIFICATION_QUEUE, SNOS_JOB_PROCESSING_QUEUE, SNOS_JOB_VERIFICATION_QUEUE,
    UPDATE_STATE_JOB_PROCESSING_QUEUE, UPDATE_STATE_JOB_VERIFICATION_QUEUE, WORKER_TRIGGER_QUEUE,
};
use crate::queue::{QueueConfig, QueueProvider};
use crate::setup::SetupConfig;

pub struct SqsQueue;

lazy_static! {
    /// Maps Queue Name to Env var of queue URL.
    pub static ref QUEUE_NAME_TO_ENV_VAR_MAPPING: HashMap<&'static str, &'static str> = HashMap::from([
        (DATA_SUBMISSION_JOB_PROCESSING_QUEUE, "SQS_DATA_SUBMISSION_JOB_PROCESSING_QUEUE_URL"),
        (DATA_SUBMISSION_JOB_VERIFICATION_QUEUE, "SQS_DATA_SUBMISSION_JOB_VERIFICATION_QUEUE_URL"),
        (PROOF_REGISTRATION_JOB_PROCESSING_QUEUE, "SQS_PROOF_REGISTRATION_JOB_PROCESSING_QUEUE_URL"),
        (PROOF_REGISTRATION_JOB_VERIFICATION_QUEUE, "SQS_PROOF_REGISTRATION_JOB_VERIFICATION_QUEUE_URL"),
        (PROVING_JOB_PROCESSING_QUEUE, "SQS_PROVING_JOB_PROCESSING_QUEUE_URL"),
        (PROVING_JOB_VERIFICATION_QUEUE, "SQS_PROVING_JOB_VERIFICATION_QUEUE_URL"),
        (SNOS_JOB_PROCESSING_QUEUE, "SQS_SNOS_JOB_PROCESSING_QUEUE_URL"),
        (SNOS_JOB_VERIFICATION_QUEUE, "SQS_SNOS_JOB_VERIFICATION_QUEUE_URL"),
        (UPDATE_STATE_JOB_PROCESSING_QUEUE, "SQS_UPDATE_STATE_JOB_PROCESSING_QUEUE_URL"),
        (UPDATE_STATE_JOB_VERIFICATION_QUEUE, "SQS_UPDATE_STATE_JOB_VERIFICATION_QUEUE_URL"),
        (JOB_HANDLE_FAILURE_QUEUE, "SQS_JOB_HANDLE_FAILURE_QUEUE_URL"),
        (WORKER_TRIGGER_QUEUE, "SQS_WORKER_TRIGGER_QUEUE_URL"),
    ]);
}

#[allow(unreachable_patterns)]
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

    async fn create_queue<'a>(&self, queue_config: &QueueConfig<'a>, config: &SetupConfig) -> Result<()> {
        let config = match config {
            SetupConfig::AWS(config) => config,
            _ => panic!("Unsupported SQS configuration"),
        };
        let sqs_client = Client::new(config);
        let res = sqs_client.create_queue().queue_name(&queue_config.name).send().await?;
        let queue_url = res.queue_url().ok_or_else(|| eyre!("Not able to get queue url from result"))?;

        let mut attributes = HashMap::new();
        attributes.insert(QueueAttributeName::VisibilityTimeout, queue_config.visibility_timeout.to_string());

        if let Some(dlq_config) = &queue_config.dlq_config {
            let dlq_url = Self::get_queue_url_from_client(dlq_config.dlq_name, &sqs_client).await?;
            let dlq_arn = Self::get_queue_arn(&sqs_client, &dlq_url).await?;
            let policy = format!(
                r#"{{"deadLetterTargetArn":"{}","maxReceiveCount":"{}"}}"#,
                dlq_arn, &dlq_config.max_receive_count
            );
            attributes.insert(QueueAttributeName::RedrivePolicy, policy);
        }

        sqs_client.set_queue_attributes().queue_url(queue_url).set_attributes(Some(attributes)).send().await?;

        Ok(())
    }
}

impl SqsQueue {
    /// To get the queue url from the given queue name
    async fn get_queue_url_from_client(queue_name: &str, sqs_client: &Client) -> Result<String> {
        Ok(sqs_client
            .get_queue_url()
            .queue_name(queue_name)
            .send()
            .await?
            .queue_url()
            .ok_or_else(|| eyre!("Unable to get queue url from the given queue_name."))?
            .to_string())
    }

    async fn get_queue_arn(client: &Client, queue_url: &str) -> Result<String> {
        let attributes = client
            .get_queue_attributes()
            .queue_url(queue_url)
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;

        Ok(attributes.attributes().unwrap().get(&QueueAttributeName::QueueArn).unwrap().to_string())
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
