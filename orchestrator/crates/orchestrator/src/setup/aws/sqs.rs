use crate::{
    core::client::queue::sqs::SQS, core::cloud::CloudProvider, core::traits::resource::Resource, setup::queue::QUEUES,
    types::params::QueueArgs, OrchestratorError, OrchestratorResult,
};
use async_trait::async_trait;
use aws_sdk_sqs::types::QueueAttributeName;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
impl Resource for SQS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = QueueArgs;

    type CheckArgs = String;

    async fn create_setup(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config, None)),
        }
    }

    /// setup - Setup SQS queue
    /// check if queue exists, if not create it. This function will create all the queues defined in the QUEUES vector.
    /// It will also create the dead letter queues for each queue if they are configured.
    /// The dead letter queues will have the same name as the queue but with the suffix "_dlq".
    /// For example, if the queue name is "test_queue", the dead letter queue name will be "test_queue_dlq".
    /// TODO: The dead letter queues will have a visibility timeout of 300 seconds and a max receive count of 5.
    /// If the dead letter queue is not configured, the dead letter queue will not be created.
    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        for queue in QUEUES.iter() {
            let queue_name = format!("{}_{}_{}", args.prefix, queue.name, args.suffix);
            let queue_url = format!("{}/{}", args.queue_base_url, queue_name);
            if self.check_if_exists(queue_url.clone()).await? {
                tracing::info!(" ⏭️️ SQS queue already exists. Queue URL: {}", queue_url);
                continue;
            }
            let res = self.client().create_queue().queue_name(&queue_name).send().await.map_err(|e| {
                OrchestratorError::ResourceSetupError(format!(
                    "Failed to create SQS queue '{}': {}",
                    args.queue_base_url, e
                ))
            })?;
            let queue_url = res
                .queue_url()
                .ok_or_else(|| OrchestratorError::ResourceSetupError("Failed to get queue url".to_string()))?;

            let mut attributes = HashMap::new();
            attributes.insert(QueueAttributeName::VisibilityTimeout, queue.visibility_timeout.to_string());

            if let Some(dlq_config) = &queue.dlq_config {
                let dlq_url = self.get_queue_url_from_client(&queue_name).await?;
                let dlq_arn = self.get_queue_arn(&dlq_url).await?;
                let policy = format!(
                    r#"{{"deadLetterTargetArn":"{}","maxReceiveCount":"{}"}}"#,
                    dlq_arn, &dlq_config.max_receive_count
                );
                attributes.insert(QueueAttributeName::RedrivePolicy, policy);
            }

            self.client().set_queue_attributes().queue_url(queue_url).set_attributes(Some(attributes)).send().await?;
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
    async fn check_if_exists(&self, queue_url: Self::CheckArgs) -> OrchestratorResult<bool> {
        Ok(self.client().get_queue_attributes().queue_url(queue_url).send().await.is_ok())
    }

    async fn is_ready_to_use(&self, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        let client = self.client().clone();
        for queue in QUEUES.iter() {
            let queue_name = format!("{}_{}_{}", args.prefix, queue.name, args.suffix);
            let queue_url = format!("{}/{}", args.queue_base_url, queue_name);
            let result = client.get_queue_attributes().queue_url(queue_url).send().await;
            if result.is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
