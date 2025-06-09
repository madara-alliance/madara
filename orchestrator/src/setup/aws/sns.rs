use crate::cli::Layer;
use crate::core::client::alert::sns::InnerAWSSNS;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::params::AWSResourceIdentifier;
use crate::types::params::AlertArgs;
use crate::{OrchestratorError, OrchestratorResult};
use anyhow::Context;
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
impl Resource for InnerAWSSNS {
    type SetupResult = ();
    type CheckResult = bool;
    type TeardownResult = ();
    type Error = ();
    type SetupArgs = AlertArgs;
    type CheckArgs = AWSResourceIdentifier;

    async fn create_setup(provider: Arc<CloudProvider>) -> OrchestratorResult<Self> {
        match provider.as_ref() {
            CloudProvider::AWS(aws_config) => Ok(Self::new(aws_config)),
        }
    }

    async fn setup(&self, _layer: &Layer, args: Self::SetupArgs) -> OrchestratorResult<Self::SetupResult> {
        let alert_name = match &args.alert_identifier {
            AWSResourceIdentifier::ARN(arn) => arn.resource.clone(),
            AWSResourceIdentifier::Name(name) => name.to_string(),
        };
        tracing::info!("Topic Name: {}", alert_name);

        // Validate topic name before proceeding
        if !self.is_valid_topic_name(&alert_name) {
            return Err(OrchestratorError::ResourceSetupError(format!(
                "Invalid topic name: {}. Topic names must be made up of letters, numbers, hyphens, and underscores.",
                alert_name
            )));
        }

        // Check if a topic exists using ARN
        if self.check_if_exists(&args.alert_identifier).await? {
            tracing::warn!(" ⏭️ SNS topic already exists. Topic Name: {}", alert_name);
            return Ok(());
        }

        // We use ARN in setup only to validate the existence of resource.
        match &args.alert_identifier {
            AWSResourceIdentifier::ARN(_) => {
                tracing::info!("Alert Arn provided, skipping setup");
                Ok(())
            }
            AWSResourceIdentifier::Name(_) => {
                // Create topic using the validated name
                let response =
                    self.client().create_topic().name(alert_name).send().await.context("Failed to create topic")?;

                let new_topic_arn = response.topic_arn().context("Failed to get topic ARN")?;
                tracing::info!("SNS topic created. Topic ARN: {}", new_topic_arn);
                Ok(())
            }
        }
    }

    async fn check_if_exists(&self, alert_identifier: &Self::CheckArgs) -> OrchestratorResult<bool> {
        match alert_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                Ok(self.client().get_topic_attributes().topic_arn(arn.to_string()).send().await.is_ok())
            }
            AWSResourceIdentifier::Name(name) => Ok(self.fetch_topic_arn_by_name_via_client(name).await.is_ok()),
        }
    }

    async fn is_ready_to_use(&self, _layer: &Layer, args: &Self::SetupArgs) -> OrchestratorResult<bool> {
        Ok(self.check_if_exists(&args.alert_identifier).await?)
    }
}
