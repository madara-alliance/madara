use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;

use super::AlertError;
use crate::{core::client::alert::AlertClient, types::params::{AWSResourceIdentifier, AlertArgs, ARN}};
use std::sync::{Arc, OnceLock};

/// AWSS3 is a struct that represents an AWS S3 client.
#[derive(Clone, Debug)]
pub(crate) struct InnerAWSSNS(pub(crate) Arc<Client>);

impl InnerAWSSNS {
    /// Creates a new instance of InnerAWSS3 with the provided AWS configuration.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    ///
    /// # Returns
    /// * `Self` - The new instance of InnerAWSS3.
    pub fn new(aws_config: &SdkConfig) -> Self {
        Self(Arc::new(Client::new(aws_config)))
    }
    pub fn client(&self) -> &Client {
        self.0.as_ref()
    }

    /// get_topic_arn return the topic name, if empty it will return an error
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic arn.
    pub async fn get_topic_arn_by_name(&self, topic_name: &String) -> Result<ARN, AlertError> {
        // Lookup Alerts from AWS...
        let resp = self.client().list_topics().send().await.map_err(AlertError::ListTopicsError)?;

        for topic in resp.topics() {
            if let Some(arn) = topic.topic_arn() {
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() == 6 && parts[5] == topic_name {
                    let arn_string = arn.to_string();
                    let arn =  ARN::parse(&arn_string).map_err(|_| {
                        tracing::debug!("ARN not parsable");
                        AlertError::TopicARNInvalid
                    })?;
                    return Ok(arn);
                }
            }
        }

        Err(AlertError::TopicNotFound(topic_name.to_string()))
    }




}

pub struct SNS {
    inner: InnerAWSSNS,
    pub alert_template_identifier: AWSResourceIdentifier,
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
        let region = aws_config.region();
        let account_id = aws_config.
        Self {
            inner: InnerAWSSNS::new(aws_config),
            alert_topic_name: args.map(|a| a.alert_topic_name.clone()),
            alert_topic_arn: Arc::new(OnceLock::new()),
        }
    }


    /// get_topic_arn return the topic name, if empty it will return an error
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic arn.
    pub async fn get_cache_topic_arn(&self) -> Result<String, AlertError> {
        // First, try to get the cached value
        if let Some(arn) = self.alert_topic_arn.get() {
            return Ok(arn.clone());
        }

        // Otherwise, resolve the ARN
        let alert_topic_name = self.alert_topic_name.clone().ok_or(AlertError::TopicARNEmpty)?;

        // If already an ARN, return it and cache it
        if alert_topic_name.starts_with("arn:") {
            let arn = alert_topic_name.clone();
            // This will only set if it hasn't been set before
            let _ = self.alert_topic_arn.set(arn.clone());
            return Ok(arn);
        }

        // Lookup ARN from AWS...
        let resp = self.client().list_topics().send().await.map_err(AlertError::ListTopicsError)?;

        for topic in resp.topics() {
            if let Some(arn) = topic.topic_arn() {
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() == 6 && parts[5] == alert_topic_name {
                    let arn_string = arn.to_string();
                    let _ = self.alert_topic_arn.set(arn_string.clone());
                    return Ok(arn_string);
                }
            }
        }

        Err(AlertError::TopicNotFound(alert_topic_name.to_string()))
    }

    pub fn client(&self) -> &Client {
        self.inner.client()
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
        self.client().publish().topic_arn(self.get_topic_arn().await?).message(message_body).send().await?;
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
