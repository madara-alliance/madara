use crate::cli::Layer;
use crate::core::client::SNS;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::params::AlertArgs;
use crate::types::Layer;
use crate::{OrchestratorError, OrchestratorResult};
use anyhow::{anyhow, Context};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
impl Resource for SNS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = AlertArgs;
    type CheckArgs = String;

    async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config, None)),
        }
    }

    async fn setup(&self, _layer: &Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        let topic_arn = args.alert_topic_name;
        tracing::info!("Topic ARN: {}", topic_arn);

        // Extract topic name from ARN or use the full string if it's just a name
        let topic_name = if topic_arn.starts_with("arn:aws:sns:") {
            topic_arn.split(':').last().ok_or_else(|| anyhow!("Invalid ARN format"))?.to_string()
        } else {
            topic_arn.clone()
        };

        // Validate topic name before proceeding
        if !self.is_valid_topic_name(&topic_name) {
            return Err(OrchestratorError::ResourceSetupError(format!(
                "Invalid topic name: {}. Topic names must be made up of letters, numbers, hyphens, and underscores.",
                topic_name
            )));
        }

        // Check if a topic exists using ARN
        if self.check_if_exists(topic_arn.clone()).await? {
            tracing::warn!(" ⏭️ SNS topic already exists. Topic ARN: {}", topic_arn);
            return Ok(());
        }

        // Create topic using the validated name
        let response = self.client.create_topic().name(topic_name).send().await.context("Failed to create topic")?;

        let new_topic_arn = response.topic_arn().context("Failed to get topic ARN")?;
        tracing::info!("SNS topic created. Topic ARN: {}", new_topic_arn);
        Ok(())
    }

    async fn check_if_exists(&self, topic_arn: Self::CheckArgs) -> OrchestratorResult<bool> {
        Ok(self.client.get_topic_attributes().topic_arn(topic_arn).send().await.is_ok())
    }

    async fn is_ready_to_use(&self, _layer: &Layer, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        Ok(self.client.get_topic_attributes().topic_arn(&args.alert_topic_name).send().await.is_ok())
    }
}

impl SNS {
    fn is_valid_topic_name(&self, name: &str) -> bool {
        // AWS SNS topic name requirements:
        // - Can include numbers, letters, hyphens, and underscores
        // - Length between 1 and 256
        if name.is_empty() || name.len() > 256 {
            return false;
        }

        name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    }
}
