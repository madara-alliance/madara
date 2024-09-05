use crate::alerts::Alerts;
use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;
use utils::env_utils::get_env_var_or_panic;

pub struct AWSSNS {
    client: Client,
}

impl AWSSNS {
    /// To create a new SNS client
    pub async fn new(config: &SdkConfig) -> Self {
        AWSSNS { client: Client::new(config) }
    }
}

#[async_trait]
impl Alerts for AWSSNS {
    async fn send_alert_message(&self, message_body: String) -> color_eyre::Result<()> {
        let topic_arn = get_env_var_or_panic("AWS_SNS_ARN");
        self.client.publish().topic_arn(topic_arn).message(message_body).send().await?;
        Ok(())
    }
}
