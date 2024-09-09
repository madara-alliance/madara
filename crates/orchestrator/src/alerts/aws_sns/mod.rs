mod config;

use crate::alerts::aws_sns::config::AWSSNSConfig;
use crate::alerts::Alerts;
use crate::config::ProviderConfig;
use async_trait::async_trait;
use aws_sdk_sns::Client;
use utils::settings::Settings;

pub const AWS_SNS_SETTINGS_NAME: &str = "sns";

pub struct AWSSNS {
    client: Client,
    topic_arn: String,
}

impl AWSSNS {
    pub async fn new_with_settings(settings: &impl Settings, provider_config: ProviderConfig) -> Self {
        match provider_config {
            ProviderConfig::AWS(aws_config) => {
                let sns_config = AWSSNSConfig::new_with_settings(settings)
                    .expect("Not able to get Aws sns config from provided settings");
                Self { client: Client::new(&aws_config), topic_arn: sns_config.sns_arn }
            }
        }
    }
}

#[async_trait]
impl Alerts for AWSSNS {
    async fn send_alert_message(&self, message_body: String) -> color_eyre::Result<()> {
        self.client.publish().topic_arn(self.topic_arn.clone()).message(message_body).send().await?;
        Ok(())
    }
}
