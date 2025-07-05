use crate::cli::Layer;
use crate::core::client::queue::sqs::InnerSQS;
use crate::types::params::AWSResourceIdentifier;
use crate::types::queue::QueueType;
use crate::types::queue_control::QUEUES;
use crate::{
    core::cloud::CloudProvider, core::traits::resource::Resource, types::params::QueueArgs,
    OrchestratorError, OrchestratorResult,
};
use async_trait::async_trait;
use aws_sdk_sqs::types::QueueAttributeName;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
impl Resource for InnerSQS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = QueueArgs;

    type CheckArgs = (AWSResourceIdentifier, QueueType);

    async fn create_setup(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config)),
        }
    }

    /// setup - Setup SQS queue
    /// check if queue exists, if not create it. This function will create all the queues defined in the QUEUES vector.
    /// It will also create the dead letter queues for each queue if they are configured.
    /// The dead letter queues will have the same name as the queue but with the suffix "_dlq".
    /// For example, if the queue name is "test_queue", the dead letter queue name will be "test_queue_dlq".
    /// TODO: The dead letter queues will have a visibility timeout of 300 seconds and a max receive count of 5.
    /// If the dead letter queue is not configured, the dead letter queue will not be created.
    async fn setup(&self, layer: &Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        for (queue_type, queue) in QUEUES.iter() {
            if !queue.supported_layers.contains(layer) {
                continue;
            }

            // Good first issue to resolve!
            // It is to note that we skip just after we check if queue exists,
            // Ideally we would want to check the DL queue & policy inclusion as well.
            if self.check_if_exists(&(args.queue_template_identifier.clone(), queue_type.clone())).await? {
                tracing::info!(" ⏭️️ SQS queue already exists. Queue Type: {}", queue_type);
                continue;
            }

            match &args.queue_template_identifier {
                AWSResourceIdentifier::ARN(arn) => {
                    // If ARN is provided, we just check if it exists
                    let queue_name = InnerSQS::get_queue_name_from_type(&arn.resource, queue_type);
                    tracing::info!("Queue Arn provided, skipping setup for {}", &queue_name);
                    continue;
                }

                AWSResourceIdentifier::Name(name) => {
                    let queue_name = InnerSQS::get_queue_name_from_type(name, queue_type);

                    // Create the queue
                    let res = self.client().create_queue().queue_name(&queue_name).send().await.map_err(|e| {
                        OrchestratorError::ResourceSetupError(format!(
                            "Failed to create SQS queue '{}': {}",
                            queue_name, e
                        ))
                    })?;

                    let queue_url = res
                        .queue_url()
                        .ok_or_else(|| OrchestratorError::ResourceSetupError("Failed to get queue url".to_string()))?;

                    tracing::info!("Queue created for type {}", queue_type);

                    let mut attributes = HashMap::new();
                    attributes.insert(QueueAttributeName::VisibilityTimeout, queue.visibility_timeout.to_string());

                    if let Some(dlq_config) = &queue.dlq_config {
                        if self
                            .check_if_exists(&(args.queue_template_identifier.clone(), dlq_config.dlq_name.clone()))
                            .await?
                        {
                            tracing::info!(" ⏭️️ DL queue already exists. Queue Type: {}", &dlq_config.dlq_name);
                            continue;
                        }

                        // Create the dl queue
                        let dlq_name = InnerSQS::get_queue_name_from_type(name, &dlq_config.dlq_name);
                        let dlq_res = self.client().create_queue().queue_name(&dlq_name).send().await.map_err(|e| {
                            OrchestratorError::ResourceSetupError(format!("Failed to create DLQ '{}': {}", dlq_name, e))
                        })?;

                        let dlq_url = dlq_res.queue_url().ok_or_else(|| {
                            OrchestratorError::ResourceSetupError("Failed to get dl queue url".to_string())
                        })?;

                        tracing::info!("DL Queue listed for type {}", queue_type);

                        let dlq_arn = self.get_queue_arn_from_url(dlq_url).await?;

                        // Attach the dl queue policy to the queue
                        let policy = format!(
                            r#"{{"deadLetterTargetArn":"{}","maxReceiveCount":"{}"}}"#,
                            dlq_arn, &dlq_config.max_receive_count
                        );
                        attributes.insert(QueueAttributeName::RedrivePolicy, policy);
                    }
                    self.client()
                        .set_queue_attributes()
                        .queue_url(queue_url)
                        .set_attributes(Some(attributes))
                        .send()
                        .await?;
                    tracing::info!("Setup completed for queue: {}", queue_type);
                }
            }
        }

        Ok(())
    }

    /// check_if_exists - Check if SQS queue exists
    /// check if queue exists by calling the get_queue_url function
    /// if the queue exists, return true
    ///
    /// # Arguments
    /// * `queue_url` - The queue url to check
    ///
    /// # Returns
    /// * `OrchestratorResult<bool>` - true if the queue exists, false otherwise
    ///
    async fn check_if_exists(&self, check_args: &Self::CheckArgs) -> OrchestratorResult<bool> {
        match check_args.0.clone() {
            AWSResourceIdentifier::ARN(arn) => {
                let queue_url = self.get_queue_url_from_arn(&arn, &check_args.1)?;
                Ok(self
                    .client()
                    .get_queue_attributes()
                    .queue_url(queue_url)
                    .attribute_names(QueueAttributeName::QueueArn)
                    .send()
                    .await
                    .is_ok())
            }
            AWSResourceIdentifier::Name(name) => {
                let queue_name = InnerSQS::get_queue_name_from_type(&name, &check_args.1);
                Ok(self.client().get_queue_url().queue_name(queue_name).send().await.is_ok())
            }
        }
    }

    async fn is_ready_to_use(&self, layer: &Layer, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        for (queue_type, queue) in QUEUES.iter() {
            if !queue.supported_layers.contains(layer) {
                continue;
            }
            let queue_exists = match &args.queue_template_identifier {
                AWSResourceIdentifier::ARN(arn) => {
                    let queue_url = self.get_queue_url_from_arn(arn, queue_type)?;
                    self.client()
                        .get_queue_attributes()
                        .queue_url(queue_url)
                        .attribute_names(QueueAttributeName::QueueArn)
                        .send()
                        .await
                        .is_ok()
                }
                AWSResourceIdentifier::Name(name) => {
                    let queue_name = InnerSQS::get_queue_name_from_type(name, queue_type);
                    match self.get_queue_url_from_client(queue_name.as_str()).await {
                        Ok(queue_url) => self.client().get_queue_attributes().queue_url(queue_url).send().await.is_ok(),
                        Err(_) => false,
                    }
                }
            };

            // If any queue doesn't exist, return false immediately
            if !queue_exists {
                return Ok(false);
            }
        }

        // All queues exist
        Ok(true)
    }
}
