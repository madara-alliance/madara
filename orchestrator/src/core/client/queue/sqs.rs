use crate::core::client::queue::QueueError;
use crate::types::constant::get_version_string;
use crate::types::params::AWSResourceIdentifier;
use crate::types::params::ARN;
use crate::{
    core::client::queue::QueueClient,
    types::{params::QueueArgs, queue::QueueType},
};
use async_trait::async_trait;
use aws_config::Region;
use aws_config::SdkConfig;
use aws_sdk_sqs::types::{MessageAttributeValue, QueueAttributeName};
use aws_sdk_sqs::Client;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::Delivery;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct InnerSQS(Client);

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
        Self(client)
    }

    pub fn client(&self) -> &Client {
        &self.0
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

        // Handle different AWS partitions
        let domain = "amazonaws.com";

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

    /// get_dlq_url_from_queue - Get the DLQ URL for a given queue
    /// This function retrieves the dead letter queue URL configured for a queue
    pub async fn get_dlq_url_from_queue(&self, queue_url: &str) -> Result<Option<String>, QueueError> {
        let attributes = self
            .client()
            .get_queue_attributes()
            .queue_url(queue_url)
            .attribute_names(QueueAttributeName::RedrivePolicy)
            .send()
            .await?;

        if let Some(attrs) = attributes.attributes() {
            if let Some(redrive_policy) = attrs.get(&QueueAttributeName::RedrivePolicy) {
                // Parse the redrive policy JSON to extract DLQ ARN
                if let Ok(policy) = serde_json::from_str::<serde_json::Value>(redrive_policy) {
                    if let Some(dlq_arn) = policy.get("deadLetterTargetArn").and_then(|v| v.as_str()) {
                        // Parse the ARN and construct DLQ URL
                        let arn = ARN::parse(dlq_arn)
                            .map_err(|_| QueueError::InvalidArn(format!("Invalid DLQ ARN: {}", dlq_arn)))?;

                        let dlq_url =
                            format!("https://sqs.{}.amazonaws.com/{}/{}", arn.region, arn.account_id, arn.resource);
                        return Ok(Some(dlq_url));
                    }
                }
            }
        }
        Ok(None)
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
                    // If ARN has empty region, use provided config
                    aws_config.clone()
                }
            }
            AWSResourceIdentifier::Name(_) => {
                // Use provided config for name-based identifier
                aws_config.clone()
            }
        };

        Self {
            inner: InnerSQS::new(&latest_aws_config),
            queue_template_identifier: args.queue_template_identifier.clone(),
        }
    }

    pub fn client(&self) -> &Client {
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
    /// **send_message** - Send a message to the queue with version metadata
    /// This function sends a message to standard SQS queues with version information
    /// stored in message attributes for version-based filtering on the consumer side.
    /// It returns a Result<(), QueueError> indicating whether the operation was successful or not
    async fn send_message(&self, queue: QueueType, payload: String, delay: Option<Duration>) -> Result<(), QueueError> {
        let queue_name = self.get_queue_name(&queue)?;
        let queue_url = self.inner.get_queue_url_from_client(queue_name.as_str()).await?;

        let version = get_version_string();

        let version_attribute = MessageAttributeValue::builder()
            .data_type("String")
            .string_value(&version)
            .build()
            .map_err(|e| QueueError::MessageAttributeError(format!("Failed to build message attribute: {}", e)))?;

        let mut send_message_request = self
            .inner
            .client()
            .send_message()
            .queue_url(&queue_url)
            .message_body(&payload)
            .message_attributes("OrchestratorVersion", version_attribute);

        if let Some(delay_duration) = delay {
            send_message_request = send_message_request.delay_seconds(delay_duration.as_secs() as i32);
        }

        send_message_request.send().await?;

        tracing::debug!("Sent message to queue {} with version {}", queue_name, version);

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
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: false }).build_producer().await?;
        Ok(producer)
    }

    /// get_consumer - Get the consumer for the given queue
    /// This function returns the consumer for the given queue.
    /// The consumer is used to receive messages from the queue.
    async fn get_consumer(&self, queue: QueueType) -> Result<SqsConsumer, QueueError> {
        let queue_name = self.get_queue_name(&queue)?;
        tracing::debug!("Getting queue url for queue name {}", queue_name);
        let queue_url = self.inner.get_queue_url_from_client(queue_name.as_str()).await?;
        tracing::debug!("Found queue url {}", queue_url);
        let consumer =
            SqsBackend::builder(SqsConfig { queue_dsn: queue_url, override_endpoint: false }).build_consumer().await?;
        Ok(consumer)
    }
    /// Consume a message from the queue with version verification.
    ///
    /// This implementation filters messages by orchestrator version:
    /// - Messages without version attributes are accepted (backward compatibility)
    /// - Messages with matching version are accepted
    /// - Messages with mismatched versions are nacked (visibility timeout set to 0)
    ///
    /// The loop continues until a valid message is found or an error occurs (including NoData).
    async fn consume_message_from_queue(&self, queue: QueueType) -> Result<Delivery, QueueError> {
        let queue_name = self.get_queue_name(&queue)?;
        let queue_url = self.inner.get_queue_url_from_client(queue_name.as_str()).await?;
        let local_version = get_version_string();

        loop {
            // Receive message using AWS SDK to access attributes
            let receive_result = self
                .inner
                .client()
                .receive_message()
                .queue_url(&queue_url)
                .message_attribute_names("OrchestratorVersion")
                .max_number_of_messages(1)
                .wait_time_seconds(20)
                .send()
                .await?;

            let messages = receive_result.messages();

            // Handle no-data scenario - return the error to caller
            if messages.is_empty() {
                continue;
            }

            let message = &messages[0];
            let receipt_handle = message
                .receipt_handle()
                .ok_or_else(|| QueueError::MessageAttributeError("Missing receipt handle".to_string()))?;

            // Check version attribute
            let should_accept = match message.message_attributes() {
                Some(attrs) => match attrs.get("OrchestratorVersion") {
                    Some(version_attr) => match version_attr.string_value() {
                        Some(msg_version) => {
                            tracing::debug!("Message version: {}, Local version: {}", msg_version, local_version);
                            msg_version == local_version
                        }
                        None => {
                            tracing::warn!("OrchestratorVersion attribute exists but has no value");
                            true // Accept if attribute exists but has no value
                        }
                    },
                    None => {
                        tracing::debug!(
                            "No OrchestratorVersion attribute found, accepting message (backward compatibility)"
                        );
                        true // Accept messages without version attribute
                    }
                },
                None => {
                    tracing::debug!("No message attributes, accepting message (backward compatibility)");
                    true // Accept messages without any attributes
                }
            };

            if should_accept {
                // Make the message immediately available again by setting visibility to 0
                // so omniqueue can consume it
                self.inner
                    .client()
                    .change_message_visibility()
                    .queue_url(&queue_url)
                    .receipt_handle(receipt_handle)
                    .visibility_timeout(0)
                    .send()
                    .await?;

                tracing::debug!("Accepted message with matching version, making it available for omniqueue");

                // Now consume via omniqueue - it should get the message we just made available
                let mut consumer = self.get_consumer(queue).await?;
                return Ok(consumer.receive().await?);
            } else {
                // Delete from processing queue and re-enqueue to DLQ to preserve the message
                // while preventing it from blocking the main queue or triggering automatic DLQ
                // after exceeding maxReceiveCount.
                tracing::warn!(
                    "Version mismatch detected. Deleting message from processing queue and re-enqueueing to DLQ for investigation."
                );

                // Get the message body to re-enqueue
                let message_body = message.body().unwrap_or("");

                // Delete from the processing queue
                self.inner
                    .client()
                    .delete_message()
                    .queue_url(&queue_url)
                    .receipt_handle(receipt_handle)
                    .send()
                    .await?;

                // Try to get DLQ URL and re-enqueue the message
                if let Some(dlq_url) = self.inner.get_dlq_url_from_queue(&queue_url).await? {
                    // Re-enqueue to DLQ with version mismatch context
                    let version_mismatch_attr = MessageAttributeValue::builder()
                        .data_type("String")
                        .string_value("VersionMismatch")
                        .build()
                        .map_err(|e| QueueError::MessageAttributeError(format!("Failed to build attribute: {}", e)))?;

                    self.inner
                        .client()
                        .send_message()
                        .queue_url(&dlq_url)
                        .message_body(message_body)
                        .message_attributes("FailureReason", version_mismatch_attr)
                        .send()
                        .await?;

                    tracing::info!("Successfully re-enqueued version-mismatched message to DLQ");
                } else {
                    tracing::warn!(
                        "No DLQ configured for queue. Version-mismatched message was deleted without re-enqueueing."
                    );
                }

                // Continue the loop to get the next message
            }
        }
    }
}
