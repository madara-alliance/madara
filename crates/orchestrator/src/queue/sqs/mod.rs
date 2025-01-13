use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::{Delivery, QueueError};
use serde::Serialize;
use url::Url;

use super::QueueType;
use crate::queue::{QueueConfig, QueueProvider};

#[derive(Debug, Clone, Serialize)]
pub struct AWSSQSValidatedArgs {
    pub queue_base_url: Url,
    pub sqs_prefix: String,
    pub sqs_suffix: String,
}

pub struct SqsQueue {
    client: Client,
    queue_base_url: Url,
    sqs_prefix: String,
    sqs_suffix: String,
}

impl SqsQueue {
    pub fn new_with_args(params: AWSSQSValidatedArgs, aws_config: &SdkConfig) -> Self {
        let sqs_config_builder = aws_sdk_sqs::config::Builder::from(aws_config);
        let client = Client::from_conf(sqs_config_builder.build());
        Self {
            client,
            queue_base_url: params.queue_base_url,
            sqs_prefix: params.sqs_prefix,
            sqs_suffix: params.sqs_suffix,
        }
    }

    pub fn get_queue_url(&self, queue_type: QueueType) -> String {
        let name = format!("{}/{}", self.queue_base_url, self.get_queue_name(queue_type));
        name
    }

    pub fn get_queue_name(&self, queue_type: QueueType) -> String {
        format!("{}_{}_{}", self.sqs_prefix, queue_type, self.sqs_suffix)
    }
}

#[allow(unreachable_patterns)]
#[async_trait]
impl QueueProvider for SqsQueue {
    async fn send_message_to_queue(&self, queue: QueueType, payload: String, delay: Option<Duration>) -> Result<()> {
        let queue_url = self.get_queue_url(queue);
        let producer = get_producer(queue_url).await?;

        match delay {
            Some(d) => producer.send_raw_scheduled(payload.as_str(), d).await?,
            None => producer.send_raw(payload.as_str()).await?,
        }

        Ok(())
    }

    async fn consume_message_from_queue(&self, queue: QueueType) -> std::result::Result<Delivery, QueueError> {
        let queue_url = self.get_queue_url(queue);
        let mut consumer = get_consumer(queue_url).await?;
        consumer.receive().await
    }

    async fn create_queue(&self, queue_config: &QueueConfig) -> Result<()> {
        let res = self.client.create_queue().queue_name(self.get_queue_name(queue_config.name.clone())).send().await?;
        let queue_url = res.queue_url().ok_or_else(|| eyre!("Not able to get queue url from result"))?;

        let mut attributes = HashMap::new();
        attributes.insert(QueueAttributeName::VisibilityTimeout, queue_config.visibility_timeout.to_string());

        if let Some(dlq_config) = &queue_config.dlq_config {
            let dlq_url = Self::get_queue_url_from_client(
                self.get_queue_name(dlq_config.dlq_name.clone()).as_str(),
                &self.client,
            )
            .await?;
            let dlq_arn = Self::get_queue_arn(&self.client, &dlq_url).await?;
            let policy = format!(
                r#"{{"deadLetterTargetArn":"{}","maxReceiveCount":"{}"}}"#,
                dlq_arn, &dlq_config.max_receive_count
            );
            attributes.insert(QueueAttributeName::RedrivePolicy, policy);
        }

        self.client.set_queue_attributes().queue_url(queue_url).set_attributes(Some(attributes)).send().await?;

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
