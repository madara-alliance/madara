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
    /// get_topic_arn return the topic name, if empty it will return error
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic arn.
    pub fn get_topic_arn(&self) -> Result<String, AlertError> {
        self.topic_arn.clone().ok_or(AlertError::TopicARNEmpty)
    }
}

#[async_trait]
impl AlertClient for SNS {
    /// send_message sends a message to the SNS topic.
    ///
    /// # Arguments
    ///
    /// * `message_body` - The message body to send.
    ///
    /// # Returns
    ///
    /// * `Result<(), AlertError>` - The result of the send operation.
    async fn send_message(&self, message_body: String) -> Result<(), AlertError> {
        let topic_name = self.get_topic_arn()?;
        self.client
            .publish()
            .topic_arn(topic_name)
            .message(message_body)
            .send()
            .await
            .map_err(|e| AlertError::SendFailure(e.to_string()))?;
        Ok(())
    }

    /// get_topic_name gets the topic name from the SNS topic ARN.
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic name.
    async fn get_topic_name(&self) -> Result<String, AlertError> {
        Ok(self.get_topic_arn()?
            .split(":")
            .last()
            .ok_or_else(|| AlertError::UnableToExtractTopicName)?
            .to_string())
    }
}
