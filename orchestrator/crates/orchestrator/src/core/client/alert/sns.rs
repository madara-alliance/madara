use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;

use super::AlertError;
use crate::{core::client::alert::AlertClient, types::params::AlertArgs};

pub struct SNS {
    client: Client,
    topic_arn: String,
}

impl SNS {
    pub fn create(args: &AlertArgs, aws_config: &SdkConfig) -> Self {
        Self { client: Client::new(aws_config), topic_arn: args.endpoint.clone() }
    }
}

#[async_trait]
impl AlertClient for SNS {
    async fn send_message(&self, message_body: String) -> Result<(), AlertError> {
        self.client
            .publish()
            .topic_arn(self.topic_arn.clone())
            .message(message_body)
            .send()
            .await
            .map_err(|e| AlertError::SendFailure(e.to_string()))?;
        Ok(())
    }

    async fn get_topic_name(&self) -> Result<String, AlertError> {
        Ok(self.topic_arn.clone().split(":").last().ok_or_else(|| AlertError::UnableToExtractTopicName)?.to_string())
    }
}
