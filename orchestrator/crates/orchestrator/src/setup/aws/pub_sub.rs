use crate::core::client::SNS;
// use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::params::AlertArgs;
use crate::{OrchestratorError, OrchestratorResult};
use anyhow::Context;
use async_trait::async_trait;
// use std::sync::Arc;

#[async_trait]
impl Resource for SNS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = AlertArgs;
    type CheckArgs = ();

    // async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
    //     match provider.as_ref() {
    //         CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config, None)),
    //     }
    // }

    async fn setup(&self) -> OrchestratorResult<Self::SetupResult> {
        // TODO: We would want to skip the setup if the bucket_identifier is an ARN !

        let alert_topic_name = self.topic_name.clone();
        tracing::info!("Topic Name: {}", alert_topic_name);

        // Validate topic name before proceeding
        if !self.is_valid_topic_name(&alert_topic_name) {
            return Err(OrchestratorError::ResourceSetupError(format!(
                "Invalid topic name: {}. Topic names must be made up of letters, numbers, hyphens, and underscores.",
                alert_topic_name
            )));
        }

        // Check if a topic exists using ARN
        if self.check_if_exists(()).await? {
            tracing::warn!(" ⏭️ SNS topic already exists.");
            return Ok(());
        }

        // Create topic using the validated name
        let response =
            self.client.create_topic().name(alert_topic_name).send().await.context("Failed to create topic")?;

        let new_topic_arn = response.topic_arn().context("Failed to get topic ARN")?;
        tracing::info!("SNS topic created. Topic ARN: {}", new_topic_arn);
        Ok(())
    }

    async fn check_if_exists(&self, _args: Self::CheckArgs) -> OrchestratorResult<bool> {
        // Ok(self.client.get_topic_attributes().topic_arn(alert_topic_arn).send().await.is_ok())
        Ok(self.get_topic_arn().await.is_ok())
    }

    async fn is_ready_to_use(&self, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        Ok(self.client.get_topic_attributes().topic_arn(&args.topic_identifier).send().await.is_ok())
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
