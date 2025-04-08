use crate::{
    core::client::queue::sqs::SQS, core::cloud::CloudProvider, core::traits::resource::Resource, setup::queue::QUEUES,
    types::params::QueueArgs, OrchestratorError, OrchestratorResult,
};
use async_trait::async_trait;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
impl Resource for SQS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = QueueArgs;

    type CheckArgs = (QueueArgs, String);

    async fn new(cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match cloud_provider.as_ref() {
            CloudProvider::AWS(aws_config) => {
                let client = Client::new(&aws_config);
                Ok(Self::constructor(client, None, None, None))
            }
            _ => Err(OrchestratorError::InvalidCloudProviderError(format!(
                "Miss match Cloud Provider {:?}",
                cloud_provider
            ))),
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
            let queue_name = format!("{}_{}_{}", args.prefix.clone(), queue.name.clone(), args.suffix.clone());
            let res = self.client().create_queue().queue_name(queue_name.clone()).send().await.map_err(|e| {
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
                let dlq_url = self.get_queue_url_from_client(queue_name.clone().as_str()).await?;
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

    async fn check(&self, _args: Self::CheckArgs) -> OrchestratorResult<Self::CheckResult> {
        let (args, queue_name) = _args;
        let queue_url = format!(
            "{}/{}_{}_{}",
            args.queue_base_url.clone(),
            args.prefix.clone(),
            queue_name.clone(),
            args.suffix.clone()
        );
        let _res = self
            .client()
            .get_queue_attributes()
            .queue_url(&queue_url)
            .attribute_names(QueueAttributeName::All)
            .send()
            .await
            .map_err(|e| {
                OrchestratorError::ResourceSetupError(format!(
                    "Failed to create SQS queue '{}': {}",
                    args.queue_base_url, e
                ))
            })?;

        Ok(true)
    }

    async fn teardown(&self) -> OrchestratorResult<()> {
        let queue_url = self
            .queue_url()
            .ok_or_else(|| OrchestratorError::ResourceError("Failed to access queue URL".to_string()))?;

        self.client()
            .delete_queue()
            .queue_url(queue_url.to_string())
            .send()
            .await
            .map_err(|e| OrchestratorError::ResourceError(format!("Failed to delete SQS queue: {}", e)))?;
        Ok(())
    }
}
