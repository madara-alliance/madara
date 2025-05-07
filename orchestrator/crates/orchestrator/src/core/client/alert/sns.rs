use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;

use super::AlertError;
use crate::{core::client::alert::AlertClient, types::params::AlertArgs};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SNS {
    pub client: Arc<Client>,
    pub topic_name: Option<String>,
    pub topic_arn: Arc<Mutex<Option<String>>>,
}

impl SNS {
    /// Since the SNS client with both client and option for the client, we've needed to pass the
    /// aws_config and args to the constructor.
    ///
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The alert arguments.
    ///
    /// # Returns
    /// * `Self` - The SNS client.
    pub(crate) fn new(aws_config: &SdkConfig, args: Option<&AlertArgs>) -> Self {
        Self {
            client: Arc::new(Client::new(aws_config)),
            topic_name: args.map(|a| a.topic_name.clone()),
            topic_arn: Arc::new(Mutex::new(None)),
        }
    }

    /// get_topic_arn return the topic name, if empty it will return an error
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic arn.
    pub async fn get_topic_arn(&self) -> Result<String, AlertError> {
        // Check if topic_arn is available
        if let Some(topic_arn) = self.topic_arn.lock().unwrap().clone() {
            return Ok(topic_arn);
        }

        let topic_name = self.topic_name.clone().ok_or(AlertError::TopicARNEmpty)?;

        // If already an ARN, return it and cache it
        if topic_name.starts_with("arn:") {
            let arn = topic_name.clone();
            // Set the cached value
            self.topic_arn
                .lock()
                .map_err(|e| AlertError::LockError(format!("Failed to acquire lock: {}", e)))
                .map(|mut guard| *guard = Some(arn.clone()))?;
            return Ok(arn);
        }

        // Rest of implementation...
        let resp = self.client.list_topics().send().await.map_err(|e| AlertError::ListTopicsError(e))?;

        let topics = resp.topics();
        for topic in topics {
            if let Some(arn) = topic.topic_arn() {
                // ARN format: arn:aws:sns:region:account-id:topic-name
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() == 6 && parts[5] == topic_name {
                    let arn_string = arn.to_string();
                    *self.topic_arn.lock().unwrap() = Some(arn_string.clone());
                    return Ok(arn_string);
                }
            }
        }

        Err(AlertError::TopicNotFound(topic_name.to_string()))
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
        self.client.publish().topic_arn(self.get_topic_arn().await?).message(message_body).send().await?;
        Ok(())
    }

    /// get_topic_name gets the topic name from the SNS topic ARN.
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic name.
    async fn get_topic_name(&self) -> Result<String, AlertError> {
        Ok(self
            .get_topic_arn()
            .await?
            .split(":")
            .last()
            .ok_or(AlertError::UnableToExtractTopicName(self.get_topic_arn().await?))?
            .to_string())
    }
}
