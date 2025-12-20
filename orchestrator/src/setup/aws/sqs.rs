use crate::core::client::queue::sqs::InnerSQS;
use crate::types::params::AWSResourceIdentifier;
use crate::types::queue::QueueType;
use crate::types::Layer;
use crate::{
    core::cloud::CloudProvider, core::traits::resource::Resource, types::params::QueueArgs,
    types::queue_control::QUEUES, OrchestratorResult,
};
use async_trait::async_trait;
use aws_sdk_sqs::types::QueueAttributeName;
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
    /// Only creates the WorkerTrigger queue (no DLQ needed for trigger queue)
    async fn setup(&self, layer: &Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        for (queue_type, queue) in QUEUES.iter() {
            if !queue.supported_layers.contains(layer) {
                continue;
            }

            // Check if queue already exists
            if self.check_if_exists(&(args.queue_template_identifier.clone(), queue_type.clone())).await? {
                tracing::info!("SQS queue already exists. Queue Type: {}", queue_type);
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

                    // Creating the queue (WorkerTrigger only, no DLQ)
                    let _queue_url = self.create_queue(queue_name.clone(), queue.visibility_timeout).await?;

                    tracing::info!("Queue created for type {}", queue_type);
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
