use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;
use std::sync::Arc;

use super::AlertError;
use crate::{core::client::alert::AlertClient, types::params::AlertArgs};

pub struct SNS {
    pub client: Arc<Client>,
    pub topic_arn: Option<String>,
}

impl SNS {
    pub(crate) fn constructor(client: Arc<Client>) -> Self {
        Self { client, topic_arn: None }
    }
    pub fn create(args: &AlertArgs, aws_config: &SdkConfig) -> Self {
        Self { client: Arc::new(Client::new(aws_config)), topic_arn: Some(args.endpoint.clone()) }
    }
}

#[async_trait]
impl AlertClient for SNS {
    async fn send_message(&self, message_body: String) -> Result<(), AlertError> {
        self.client
            .publish()
            .topic_arn(self.topic_arn.clone().ok_or_else(|| AlertError::UnableToExtractTopicName)?)
            .message(message_body)
            .send()
            .await
            .map_err(|e| AlertError::SendFailure(e.to_string()))?;
        Ok(())
    }

    async fn get_topic_name(&self) -> Result<String, AlertError> {
        Ok(self
            .topic_arn
            .clone()
            .ok_or_else(|| AlertError::UnableToExtractTopicName)?
            .split(":")
            .last()
            .ok_or_else(|| AlertError::UnableToExtractTopicName)?
            .to_string())
    }
}
