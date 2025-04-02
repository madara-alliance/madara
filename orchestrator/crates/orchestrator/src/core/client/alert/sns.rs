use crate::core::client::alert::AlertClient;
use crate::params::AlertArgs;
use crate::OrchestratorResult;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;

pub struct SNS {
    client: Client,
    topic_arn: String,
}

impl SNS {
    pub fn setup(args: &AlertArgs, aws_config: &SdkConfig) -> Self {
        Self { client: Client::new(aws_config), topic_arn: args.endpoint.clone() }
    }
}

#[async_trait]
impl AlertClient for SNS {
    async fn create_alert(&self, topic_name: &str) -> OrchestratorResult<()> {
        // TODO: Implement SNS topic creation
        Ok(())
    }

    async fn get_topic_name(&self) -> String {
        self.topic_arn.clone()
    }

    async fn send_alert_message(&self, message_body: String) -> OrchestratorResult<()> {
        // TODO: Implement SNS message sending
        Ok(())
    }
}
