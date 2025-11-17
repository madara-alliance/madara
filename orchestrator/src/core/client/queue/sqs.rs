use crate::core::client::queue::QueueError;
use crate::types::constant::{ORCHESTRATOR_VERSION, ORCHESTRATOR_VERSION_ATTRIBUTE};
use crate::types::params::AWSResourceIdentifier;
use crate::types::params::ARN;
use crate::{
    core::client::queue::QueueClient,
    types::{params::QueueArgs, queue::QueueType},
    OrchestratorError,
};
use async_trait::async_trait;
use aws_config::Region;
use aws_config::SdkConfig;
use aws_sdk_sqs::types::{MessageAttributeValue, QueueAttributeName};
use aws_sdk_sqs::Client;
use omniqueue::backends::{SqsBackend, SqsConfig, SqsConsumer, SqsProducer};
use omniqueue::Delivery;
use std::collections::HashMap;
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

    /// get_queue_name_from_type - Get the queue specific name from its type
    /// This function returns the queue name based on the queue type provided
    pub fn get_queue_name_from_type(name: &str, queue_type: &QueueType) -> String {
        name.replace("{}", &queue_type.to_string())
    }

    /// Create a new queue with the given name
    pub async fn create_queue(&self, queue_name: String, visibility_timeout: u32) -> Result<String, OrchestratorError> {
        let mut attributes = HashMap::new();
        attributes.insert(QueueAttributeName::VisibilityTimeout, visibility_timeout.to_string());
        let res = self
            .client()
            .create_queue()
            .queue_name(&queue_name)
            .set_attributes(Some(attributes))
            .send()
            .await
            .map_err(|e| {
                OrchestratorError::ResourceSetupError(format!("Failed to create SQS queue '{}': {}", queue_name, e))
            })?;

        Ok(res
            .queue_url()
            .ok_or_else(|| OrchestratorError::ResourceSetupError("Failed to get SQS URL".to_string()))?
            .to_string())
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

        let version_attribute = MessageAttributeValue::builder()
            .data_type("String")
            .string_value(ORCHESTRATOR_VERSION)
            .build()
            .map_err(|e| QueueError::MessageAttributeError(format!("Failed to build message attribute: {}", e)))?;

        let mut send_message_request = self
            .inner
            .client()
            .send_message()
            .queue_url(&queue_url)
            .message_body(&payload)
            .message_attributes(ORCHESTRATOR_VERSION_ATTRIBUTE, version_attribute);

        if let Some(delay_duration) = delay {
            send_message_request = send_message_request.delay_seconds(delay_duration.as_secs() as i32);
        }

        send_message_request.send().await?;

        tracing::debug!("Sent message to queue {} with version {}", queue_name, ORCHESTRATOR_VERSION);

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
    /// Consume a message from the queue with client-side version filtering.
    ///
    /// # Implementation Strategy
    ///
    /// This function implements **client-side version filtering** because AWS SQS does not support
    /// server-side filtering based on message attribute values. The process is:
    ///
    /// 1. **Receive Message**: Fetch a message from SQS with all message attributes (line 265-274)
    /// 2. **Extract Version**: Read the `OrchestratorVersion` attribute from message metadata (line 284-292)
    /// 3. **Version Check**: Compare message version against current orchestrator version (line 294)
    /// 4. **Handle Mismatch**: If versions don't match, delete the original message and re-enqueue
    ///    it with the same attributes, allowing other orchestrator instances to process it (line 309-389)
    /// 5. **Process or Retry**: If compatible, wrap and return for processing; if incompatible, return NoData
    ///
    /// # Why Not Use OmniQueue Consumer
    ///
    /// OmniQueue's consumer abstraction doesn't provide access to message attributes before
    /// acknowledging receipt, which is required for our version-based routing. We need to:
    /// - Inspect message attributes before deciding whether to process
    /// - Re-enqueue messages with different versions (delete + send pattern)
    /// - Maintain message attributes when re-enqueueing for other consumers
    ///
    /// # Version Filtering Logic
    ///
    /// - **Matching version**: Returns message wrapped in OmniQueue Delivery for processing
    /// - **No version attribute**: Accepts message (backward compatibility with pre-versioning messages)
    /// - **Mismatched version**: Deletes and re-enqueues message, then returns NoData to continue polling
    ///
    /// # Re-enqueue Pattern (Delete + Send)
    ///
    /// We use delete-then-send instead of ChangeMessageVisibility(0) because:
    /// 1. Preserves message attributes across the re-enqueue operation
    /// 2. Resets the receive count (prevents premature DLQ routing for version mismatches)
    /// 3. Creates a fresh message that other version consumers can immediately receive
    ///
    /// Trade-off: This pattern bypasses receive count tracking, so messages with version
    /// mismatches won't be automatically sent to DLQ even after max retries. This is
    /// intentional - version mismatches are not processing failures.
    ///
    /// # Performance Implications
    ///
    /// This creates additional SQS API calls (ReceiveMessage -> DeleteMessage -> SendMessage)
    /// for incompatible messages. In environments with multiple orchestrator versions processing
    /// the same queue, this can increase costs and latency. Consider using separate queues
    /// per version for high-throughput scenarios.
    ///
    /// # Error Handling
    ///
    /// - If re-enqueue send fails: Returns error, message remains hidden until visibility timeout
    /// - If re-enqueue send succeeds but delete fails: Message is duplicated (better than loss)
    /// - If message is malformed (missing body/receipt_handle): Logs critical error and returns NoData
    async fn consume_message_from_queue(&self, queue: QueueType) -> Result<Delivery, QueueError> {
        let queue_name = self.get_queue_name(&queue)?;
        let queue_url = self.inner.get_queue_url_from_client(queue_name.as_str()).await?;

        let messages = self
            .inner
            .client()
            .receive_message()
            .queue_url(&queue_url)
            .max_number_of_messages(1)
            .message_attribute_names("All")
            .send()
            .await?;

        let Some(messages_vec) = messages.messages else {
            return Err(omniqueue::QueueError::NoData.into());
        };

        let Some(message) = messages_vec.first() else {
            return Err(omniqueue::QueueError::NoData.into());
        };

        let version_match = if let Some(attributes) = message.message_attributes() {
            if let Some(version_attr) = attributes.get(ORCHESTRATOR_VERSION_ATTRIBUTE) {
                version_attr.string_value() == Some(ORCHESTRATOR_VERSION)
            } else {
                true
            }
        } else {
            true
        };

        if !version_match {
            let message_version = message
                .message_attributes()
                .and_then(|attrs| attrs.get(ORCHESTRATOR_VERSION_ATTRIBUTE))
                .and_then(|v| v.string_value())
                .unwrap_or("none");

            tracing::info!(
                queue = %queue_name,
                expected_version = %ORCHESTRATOR_VERSION,
                message_version = %message_version,
                message_id = ?message.message_id(),
                "Skipping message with incompatible version - will re-enqueue for compatible orchestrator"
            );

            match (message.body(), message.receipt_handle()) {
                (Some(body), Some(receipt_handle)) => {
                    // Re-enqueue incompatible message with proper error handling
                    // Note: We use delete+send instead of ChangeMessageVisibility(0) because:
                    // 1. Preserves message attributes across the re-enqueue operation
                    // 2. Resets the receive count (prevents premature DLQ routing)
                    // 3. Creates a fresh message that other version consumers can immediately receive
                    let mut send_request = self.inner.client().send_message().queue_url(&queue_url).message_body(body);

                    if let Some(attributes) = message.message_attributes() {
                        for (key, value) in attributes {
                            send_request = send_request.message_attributes(key.clone(), value.clone());
                        }
                    }

                    // Send first - if this fails, delete won't be attempted (message preserved)
                    match send_request.send().await {
                        Ok(_) => {
                            tracing::debug!(
                                queue = %queue_name,
                                message_version = %message_version,
                                "Successfully re-enqueued incompatible message"
                            );

                            // Only delete if re-enqueue succeeded
                            if let Err(e) = self
                                .inner
                                .client()
                                .delete_message()
                                .queue_url(&queue_url)
                                .receipt_handle(receipt_handle)
                                .send()
                                .await
                            {
                                tracing::error!(
                                    queue = %queue_name,
                                    error = %e,
                                    message_version = %message_version,
                                    "Failed to delete message after successful re-enqueue - message will be duplicated"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                queue = %queue_name,
                                error = %e,
                                message_version = %message_version,
                                message_id = ?message.message_id(),
                                "CRITICAL: Failed to re-enqueue incompatible message - message may be lost after visibility timeout"
                            );
                            return Err(e.into());
                        }
                    }
                }
                (None, Some(_)) => {
                    tracing::error!(
                        queue = %queue_name,
                        message_id = ?message.message_id(),
                        message_version = %message_version,
                        "CRITICAL: Incompatible message missing body - cannot re-enqueue, message will be lost"
                    );
                }
                (Some(_), None) => {
                    tracing::error!(
                        queue = %queue_name,
                        message_id = ?message.message_id(),
                        message_version = %message_version,
                        "CRITICAL: Incompatible message missing receipt handle - cannot re-enqueue, message will be lost"
                    );
                }
                (None, None) => {
                    tracing::error!(
                        queue = %queue_name,
                        message_id = ?message.message_id(),
                        message_version = %message_version,
                        "CRITICAL: Incompatible message missing both body and receipt handle - malformed SQS message"
                    );
                }
            }

            return Err(omniqueue::QueueError::NoData.into());
        }
        let consumer = self.get_consumer(queue.clone()).await?;
        let delivery = consumer.wrap_message(messages_vec.first().unwrap());

        Ok(delivery)
    }
}
