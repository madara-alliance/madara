use crate::core::client::queue::QueueError;
use crate::{
    core::client::queue::QueueClient,
    types::{params::QueueArgs, queue::QueueType},
};
use async_trait::async_trait;
use aws_config::Region;
use aws_config::SdkConfig;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct SQS {
    pub client: Arc<Client>,
    queue_template_name: String, // The template string with "{}" placeholder for queue type
    queue_arn_base: Option<String>, // Base ARN for constructing queue ARNs
    region: Option<String>,      // Region extracted from ARN
}

impl SQS {
    /// new - Create a new SQS client with the provided AWS configuration and queue identifier.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The queue arguments, containing a queue_identifier that may be an ARN template or simple template.
    /// # Returns
    /// * `Self` - The SQS client.
    pub fn new(aws_config: &SdkConfig, args: &QueueArgs) -> Self {
        let (queue_template_name, queue_arn_base, region) = Self::parse_arn_queue_identifier(args);

        // Configure SQS client with the right region if specified in ARN
        let mut sqs_config_builder = aws_sdk_sqs::config::Builder::from(aws_config);

        // Set region from ARN if available
        if let Some(region) = &region {
            sqs_config_builder = sqs_config_builder.region(Region::new(region.clone()));
        }

        let client = Client::from_conf(sqs_config_builder.build());

        Self { client: Arc::new(client), queue_template_name, queue_arn_base, region }
    }

    /// Parse a queue identifier into template string, ARN base, and region
    /// Returns: (queue_name_with_prefix, arn_base, region)
    fn parse_arn_queue_identifier(args: &QueueArgs) -> (String, Option<String>, Option<String>) {
        let identifier = &args.queue_identifier;

        if identifier.starts_with("arn:aws:sqs:") {
            let parts: Vec<&str> = identifier.split(':').collect();

            // Standard SQS ARN has format arn:aws:sqs:{region}:{account-id}:{queue-name}
            // Note: We don't add AWS_PREFIX if we received the whole ARN !
            if parts.len() >= 6 {
                let region = parts[3].to_string();
                let account_id = parts[4].to_string();
                let queue_name = parts[5];
                let arn_base = format!("arn:aws:sqs:{}:{}", region, account_id);

                return (queue_name.to_string(), Some(arn_base), Some(region));
            }
        }

        let name = Self::transform_template_name(&args.aws_prefix, &identifier);
        // If not an ARN, just use as a queue name with prefix
        (name, None, None)
    }

    pub fn client(&self) -> Arc<Client> {
        self.client.clone()
    }

    pub fn region(&self) -> Option<String> {
        self.region.clone()
    }

    /// get_queue_name - Get the queue name by replacing the placeholder in the template
    /// The queue name is constructed by replacing {} with the queue type
    pub fn get_queue_name(&self, queue_type: &QueueType) -> Result<String, QueueError> {
        Ok(self.queue_template_name.replace("{}", &queue_type.to_string()))
    }

    /// get_queue_url - Get the queue URL
    /// This function constructs the queue URL either from the ARN base or by looking it up
    pub async fn get_queue_url(&self, queue_type: &QueueType) -> Result<String, QueueError> {
        let queue_name = self.get_queue_name(queue_type)?;
        // If we have an ARN base, we can construct the queue URL directly
        if let Some(arn_base) = &self.queue_arn_base {
            // For ARN-based queue URLs, use the format expected by AWS SQS
            if let Some(region) = &self.region {
                if let Some(account_id) = self.extract_account_id_from_arn_base(arn_base) {
                    // Format: https://sqs.{region}.amazonaws.com/{account_id}/{queue_name}
                    return Ok(format!("https://sqs.{}.amazonaws.com/{}/{}", region, account_id, queue_name));
                }
            }
        }

        // Fall back to lookup if we couldn't construct it from the ARN
        self.get_queue_url_from_client(queue_type).await
    }

    /// Extract account ID from ARN base
    fn extract_account_id_from_arn_base(&self, arn_base: &str) -> Option<String> {
        let parts: Vec<&str> = arn_base.split(':').collect();
        if parts.len() >= 5 {
            Some(parts[4].to_string())
        } else {
            None
        }
    }

    /// get_queue_url_from_client - Get the queue URL from the client
    /// This function returns the queue URL based on the queue name.
    pub async fn get_queue_url_from_client(&self, queue_type: &QueueType) -> Result<String, QueueError> {
        Ok(self
            .client()
            .get_queue_url()
            .queue_name(self.get_queue_name(queue_type)?)
            .send()
            .await?
            .queue_url()
            .ok_or_else(|| QueueError::FailedToGetQueueUrl(queue_type.to_string()))?
            .to_string())
    }

    /// get_queue_arn - Get the queue ARN from the queue URL or construct it
    pub async fn get_queue_arn(&self, queue_url: &str) -> Result<String, QueueError> {
        // If we have an ARN base, we can construct the ARN directly
        if let Some(arn_base) = &self.queue_arn_base {
            // Extract queue name from the queue URL
            let queue_name = queue_url
                .split('/')
                .last()
                .ok_or_else(|| QueueError::FailedToGetQueueArn(format!("Invalid queue URL format: {}", queue_url)))?;

            return Ok(format!("{}:{}", arn_base, queue_name));
        }

        // Fall back to looking up the ARN from queue attributes
        self.get_queue_arn_from_attributes(queue_url).await
    }
    /// Get queue ARN from queue attributes
    async fn get_queue_arn_from_attributes(&self, queue_url: &str) -> Result<String, QueueError> {
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

    /// Returns the name formed by combining AWS_PREFIX
    pub fn transform_template_name(prefix: &str, name: &str) -> String {
        format!("{}_{}", prefix, name)
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

    /// get_producer - Get the producer for the given queue
    async fn get_producer(&self, queue: QueueType) -> Result<SqsProducer, QueueError> {
        let queue_url = self.get_queue_url(&queue).await?;
        let producer =
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: true }).build_producer().await?;
        Ok(producer)
    }

    /// get_consumer - Get the consumer for the given queue
    async fn get_consumer(&self, queue: QueueType) -> Result<SqsConsumer, QueueError> {
        let queue_url = self.get_queue_url(&queue).await?;
        let consumer =
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: true }).build_consumer().await?;
        Ok(consumer)
    }

    async fn consume_message_from_queue(&self, queue: QueueType) -> Result<Delivery, QueueError> {
        let mut consumer = self.get_consumer(queue).await?;
        Ok(consumer.receive().await?)
    }
}
