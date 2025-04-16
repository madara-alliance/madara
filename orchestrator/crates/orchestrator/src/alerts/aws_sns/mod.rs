use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;

use crate::alerts::Alerts;
use crate::setup::ResourceStatus;

#[derive(Debug, Clone)]
pub struct AWSSNSValidatedArgs {
    // TODO: convert to ARN type, and validate it
    // NOTE: aws is using str to represent ARN : https://docs.aws.amazon.com/sdk-for-rust/latest/dg/rust_sns_code_examples.html
    pub topic_arn: String,
}

pub struct AWSSNS {
    client: Client,
    topic_arn: String,
}

impl AWSSNS {
    pub async fn new_with_args(aws_sns_params: &AWSSNSValidatedArgs, aws_config: &SdkConfig) -> Self {
        Self { client: Client::new(aws_config), topic_arn: aws_sns_params.topic_arn.clone() }
    }
}

#[async_trait]
impl Alerts for AWSSNS {
    async fn send_alert_message(&self, message_body: String) -> color_eyre::Result<()> {
        self.client.publish().topic_arn(self.topic_arn.clone()).message(message_body).send().await?;
        Ok(())
    }

    async fn create_alert(&self, topic_name: &str) -> color_eyre::Result<()> {
        let response = self.client.create_topic().name(topic_name).send().await?;
        let topic_arn = response.topic_arn().expect("Topic Not found");
        log::info!("SNS topic created. Topic ARN: {}", topic_arn);
        Ok(())
    }

    async fn get_topic_name(&self) -> String {
        self.topic_arn.split(":").last().expect("Failed to get last part of topic ARN").to_string()
    }

    async fn exists(&self) -> color_eyre::Result<bool> {
        let topic_name = self.get_topic_name().await;
        Ok(self.client.get_topic_attributes().topic_arn(topic_name).send().await.is_ok())
    }
}

#[allow(unreachable_patterns)]
#[async_trait]
impl ResourceStatus for AWSSNS {
    async fn are_all_ready(&self) -> bool {
        self.exists().await.is_ok()
    }
}