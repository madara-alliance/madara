use crate::core::client::queue::QueueError;
use crate::types::params::AWSResourceIdentifier;
use crate::types::params::ARN;
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
pub struct InnerSQS(pub Arc<Client>);

impl InnerSQS {
    /// Creates a new instance of InnerSQS with the provided AWS configuration.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    ///
    /// # Returns
    /// * `Self` - The new instance of InnerSQS.
    pub fn new(aws_config: &SdkConfig) -> Self {
        let sqs_config_builder = aws_sdk_sqs::config::Builder::from(aws_config);
        let client = Client::from_conf(sqs_config_builder.build());
        Self(Arc::new(client))
    }

    pub fn client(&self) -> Arc<Client> {
        self.0.clone()
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

    /// get_queue_url_from_arn - Get the queue URL from the arn
    /// This function returns the queue URL based on the queue arn.
    /// SQS queue URLs follow the format: https://sqs.{region}.amazonaws.com/{account_id}/{queue_name}
    pub fn get_queue_url_from_arn(&self, queue_arn: &ARN, queue_type: &QueueType) -> Result<String, QueueError> {
        // Validate that this is an SQS ARN
        if queue_arn.service != "sqs" {
            return Err(QueueError::InvalidArn(format!("Expected SQS ARN but got service: {}", queue_arn.service)));
        }

        // Validate required fields
        if queue_arn.region.is_empty() {
            return Err(QueueError::InvalidArn("SQS ARN must have a region".to_string()));
        }

        if queue_arn.account_id.is_empty() {
            return Err(QueueError::InvalidArn("SQS ARN must have an account ID".to_string()));
        }

        if queue_arn.resource.is_empty() {
            return Err(QueueError::InvalidArn("SQS ARN must have a queue name".to_string()));
        }

        // Handle different AWS partitions
        let domain = match queue_arn.partition.as_str() {
            // Default fallback
            _ => "amazonaws.com",
        };

        // Construct the queue URL
        let queue_url = format!(
            "https://sqs.{}.{}/{}/{}",
            queue_arn.region,
            domain,
            queue_arn.account_id,
            InnerSQS::get_queue_name_from_type(queue_arn.resource.as_str(), queue_type)
        );

        Ok(queue_url)
    }

    /// get_queue_arn - Get the queue ARN from the queue URL
    /// This function returns the queue ARN based on the queue URL.
    pub async fn get_queue_arn_from_url(&self, queue_url: &str) -> Result<ARN, QueueError> {
        let attributes = self
            .client()
            .get_queue_attributes()
            .queue_url(queue_url)
            .attribute_names(QueueAttributeName::QueueArn)
            .send()
            .await?;

        match attributes.attributes() {
            Some(attributes) => match attributes.get(&QueueAttributeName::QueueArn) {
                Some(arn) => {
                    let arn = ARN::parse(arn).map_err(|_| QueueError::FailedToGetQueueArn(queue_url.to_string()))?;
                    Ok(arn)
                }
                None => Err(QueueError::FailedToGetQueueArn(queue_url.to_string())),
            },
            None => Err(QueueError::FailedToGetQueueArn(queue_url.to_string())),
        }
    }

    /// get_queue_name_from_type - Get the queue specific name from it's type
    /// This function returns the queue name based on the queue type provided
    pub fn get_queue_name_from_type(name: &str, queue_type: &QueueType) -> String {
        name.replace("{}", &queue_type.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct SQS {
    pub inner: InnerSQS,
    queue_template_identifier: AWSResourceIdentifier,
}

impl SQS {
    /// new - Create a new SQS client, with both client and option for the client;
    /// we've needed to pass the aws_config and args to the constructor.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The queue arguments.
    /// # Returns
    /// * `Self` - The SQS client.
    pub fn new(aws_config: &SdkConfig, args: &QueueArgs) -> Self {
        let latest_aws_config = match &args.queue_template_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                // Extract region from ARN and create new config with that region
                if !arn.region.is_empty() {
                    aws_config.clone().into_builder().region(Region::new(arn.region.clone())).build()
                } else {
                    // If ARN has empty region, use original config
                    aws_config.clone()
                }
            }
            AWSResourceIdentifier::Name(_) => {
                // Use original config for name-based identifier
                aws_config.clone()
            }
        };

        Self {
            inner: InnerSQS::new(&latest_aws_config),
            queue_template_identifier: args.queue_template_identifier.clone(),
        }
    }

    pub fn client(&self) -> Arc<Client> {
        self.inner.client()
    }

    /// get_queue_name - Get the queue name
    /// This function returns the queue name based on the queue type and the queue template identifier
    /// If the identifier is an ARN, it extracts the resource name and uses it as template
    /// If the identifier is a Name, it uses it directly as template
    /// The template should contain "{}" which will be replaced with the queue type
    pub fn get_queue_name(&self, queue_type: &QueueType) -> Result<String, QueueError> {
        let template = match &self.queue_template_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                // Extract the resource name from ARN to use as template
                &arn.resource
            }
            AWSResourceIdentifier::Name(name) => {
                // Use the name directly as template
                name
            }
        };

        Ok(InnerSQS::get_queue_name_from_type(template, queue_type))
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
        let queue_name = self.get_queue_name(&queue)?;
        let queue_url = self.inner.get_queue_url_from_client(queue_name.as_str()).await?;
        let producer =
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: true }).build_producer().await?;
        Ok(producer)
    }

    /// get_consumer - Get the consumer for the given queue
    /// This function returns the consumer for the given queue.
    /// The consumer is used to receive messages from the queue.
    async fn get_consumer(&self, queue: QueueType) -> Result<SqsConsumer, QueueError> {
        let queue_name = self.get_queue_name(&queue)?;
        let queue_url = self.inner.get_queue_url_from_client(queue_name.as_str()).await?;
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
