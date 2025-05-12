use crate::core::client::queue::QueueError;
use crate::{
    core::client::queue::QueueClient,
    types::{params::QueueArgs, queue::QueueType},
};
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

#[derive(Clone, Debug)]
pub struct SQS {
    client: Arc<Client>,
    queue_url: Option<Url>,
    prefix: Option<String>,
    suffix: Option<String>,
}

impl SQS {
    /// new - Create a new SQS client, with both client and option for the client;
    /// we've needed to pass the aws_config and args to the constructor.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The queue arguments.
    /// # Returns
    /// * `Self` - The SQS client.
    pub fn new(aws_config: &SdkConfig, args: Option<&QueueArgs>) -> Self {
        let sqs_config_builder = aws_sdk_sqs::config::Builder::from(aws_config);
        let client = Client::from_conf(sqs_config_builder.build());
        Self {
            client: Arc::new(client),
            queue_url: args.and_then(|a| Url::parse(&a.queue_base_url).ok()),
            prefix: args.map(|a| a.prefix.clone()),
            suffix: args.map(|a| a.suffix.clone()),
        }
    }

    pub fn client(&self) -> Arc<Client> {
        self.client.clone()
    }
    pub fn queue_url(&self) -> Option<Url> {
        self.queue_url.clone()
    }
    pub fn prefix(&self) -> Option<String> {
        self.prefix.clone()
    }
    pub fn suffix(&self) -> Option<String> {
        self.suffix.clone()
    }

    /// get_queue_url - Get the queue URL
    /// This function returns the queue URL based on the queue type and the queue base URL
    /// The queue URL is constructed as follows:
    /// queue_base_url/queue_name
    pub fn get_queue_url(&self, queue_type: QueueType) -> Result<String, QueueError> {
        Ok(format!(
            "{}/{}",
            self.queue_url
                .clone()
                .ok_or_else(|| QueueError::MissingRootParameter("Queue URL is not set".to_string()))?,
            self.get_queue_name(queue_type)?
        ))
    }

    /// get_queue_name - Get the queue name
    /// This function returns the queue name based on the queue type and the queue prefix and suffix
    /// The queue name is constructed as follows:
    /// queue_prefix_queue_type_queue_suffix
    pub fn get_queue_name(&self, queue_type: QueueType) -> Result<String, QueueError> {
        Ok(format!(
            "{}_{}_{}",
            self.prefix
                .clone()
                .ok_or_else(|| QueueError::MissingRootParameter("Queue prefix is not set".to_string()))?,
            queue_type,
            self.suffix
                .clone()
                .ok_or_else(|| QueueError::MissingRootParameter("Queue suffix is not set".to_string()))?
        ))
    }

    /// get_queue_url_from_client - Get the queue URL from the client
    /// This function returns the queue URL based on the queue name.
    pub async fn get_queue_url_from_client(&self, queue_name: &str) -> Result<String, QueueError> {
        Ok(self
            .client()
            .get_queue_url()
            .queue_name(queue_name)
            .send()
            .await?
            .queue_url()
            .ok_or_else(|| QueueError::FailedToGetQueueUrl(queue_name.to_string()))?
            .to_string())
    }

    /// get_queue_arn - Get the queue ARN from the queue URL
    /// This function returns the queue ARN based on the queue URL.
    pub async fn get_queue_arn(&self, queue_url: &str) -> Result<String, QueueError> {
        let attributes = self
            .client()
            .get_queue_attributes()
            .queue_url(queue_url)
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;

        match attributes.attributes() {
            Some(attributes) => match attributes.get(&QueueAttributeName::QueueArn) {
                Some(arn) => Ok(arn.to_string()),
                None => Err(QueueError::FailedToGetQueueArn(queue_url.to_string())),
            },
            None => Err(QueueError::FailedToGetQueueArn(queue_url.to_string())),
        }
    }
}

#[async_trait]
impl QueueClient for SQS {
    /// **send_message** - Send a message to the queue
    /// This function sends a message to the queue.
    /// It returns a Result<(), OrchestratorError> indicating whether the operation was successful or not
    async fn send_message(&self, queue: QueueType, payload: String, delay: Option<Duration>) -> Result<(), QueueError> {
        let producer = self.get_producer(queue).await?;
        match delay {
            Some(d) => producer.send_raw_scheduled(payload.as_str(), d).await?,
            None => producer.send_raw(payload.as_str()).await?,
        }
        Ok(())
    }

    /// TODO: if possible try to reuse the same producer which got created in the previous run
    /// get_producer - Get the producer for the given queue
    /// This function returns the producer for the given queue.
    /// The producer is used to send messages to the queue.
    async fn get_producer(&self, queue: QueueType) -> Result<SqsProducer, QueueError> {
        let queue_url = self.get_queue_url(queue)?;
        let producer =
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: true }).build_producer().await?;
        Ok(producer)
    }

    /// get_consumer - Get the consumer for the given queue
    /// This function returns the consumer for the given queue.
    /// The consumer is used to receive messages from the queue.
    async fn get_consumer(&self, queue: QueueType) -> Result<SqsConsumer, QueueError> {
        let queue_url = self.get_queue_url(queue)?;
        let consumer =
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: true }).build_consumer().await?;
        Ok(consumer)
    }
    /// TODO: this should not be need remove this after reviewing the code access for usage
    async fn consume_message_from_queue(&self, queue: QueueType) -> Result<Delivery, QueueError> {
        let mut consumer = self.get_consumer(queue).await?;
        Ok(consumer.receive().await?)
    }
}
