use crate::core::client::SNS;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::params::AlertArgs;
use crate::{OrchestratorError, OrchestratorResult};
use anyhow::Context;
use async_trait::async_trait;
use aws_sdk_sns::Client as SNSClient;
use std::sync::Arc;

#[async_trait]
impl Resource for SNS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = AlertArgs;
    type CheckArgs = AlertArgs;

    async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match provider.as_ref() {
            CloudProvider::AWS(aws_config) => {
                let client = SNSClient::new(aws_config);
                Ok(Self::constructor(Arc::new(client)))
            }
            _ => Err(OrchestratorError::InvalidCloudProviderError(
                "Mismatch Cloud Provider for S3Bucket resource".to_string(),
            ))?,
        }
    }

    async fn setup(&self, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        let topic_name = args.endpoint.clone();
        tracing::info!("Topic ARN: {}", args.endpoint);
        if self.check(args.clone()).await? {
            tracing::warn!("SNS topic already exists. Topic ARN: {}", args.endpoint);
            return Ok(());
        }

        let response = self
            .client
            .create_topic()
            .name(topic_name)
            .send()
            .await
            .context("Failed to create topic")
            .expect("Failed to create topic");
        let topic_arn = response.topic_arn().context("Failed to create topic").expect("Topic Not found");
        tracing::info!("SNS topic created. Topic ARN: {}", topic_arn);
        Ok(())
    }

    async fn check(&self, args: Self::CheckArgs) -> OrchestratorResult<Self::CheckResult> {
        Ok(self.client.get_topic_attributes().topic_arn(args.endpoint).send().await.is_ok())
    }

    async fn teardown(&self) -> OrchestratorResult<()> {
        Ok(())
    }
}
