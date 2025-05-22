use async_trait::async_trait;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;
use std::sync::Arc;

use super::AlertError;
use crate::{core::client::alert::AlertClient, types::params::AlertArgs};

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
}

pub struct SNS {
    inner: InnerAWSSNS,
    pub topic_arn: Option<String>,
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
        Self { inner: InnerAWSSNS::new(aws_config), topic_arn: args.map(|a| a.endpoint.clone()) }
    }

    /// get_topic_arn return the topic name, if empty it will return an error
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic arn.
    pub fn get_topic_arn(&self) -> Result<String, AlertError> {
        self.topic_arn.clone().ok_or(AlertError::TopicARNEmpty)
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
        self.client().publish().topic_arn(self.get_topic_arn()?).message(message_body).send().await?;
        Ok(())
    }

    /// get_topic_name gets the topic name from the SNS topic ARN.
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic name.
    async fn get_topic_name(&self) -> Result<String, AlertError> {
        Ok(self
            .get_topic_arn()?
            .split(":")
            .last()
            .ok_or(AlertError::UnableToExtractTopicName(self.get_topic_arn()?))?
            .to_string())
    }
}
