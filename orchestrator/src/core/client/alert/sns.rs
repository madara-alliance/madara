use super::AlertError;
use crate::{
    core::client::alert::AlertClient,
    types::params::{AWSResourceIdentifier, AlertArgs, ARN},
};
use async_trait::async_trait;
use aws_config::Region;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;
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

    /// get_topic_arn_by_name return the topic arn, if empty it will return an error
    ///
    /// # Returns
    ///
    /// * `Result<ARN, AlertError>` - The topic arn.
    pub async fn fetch_topic_arn_by_name_via_client(&self, topic_name: &String) -> Result<ARN, AlertError> {
        let resp = self.client().list_topics().send().await.map_err(AlertError::ListTopicsError)?;

        for topic in resp.topics() {
            if let Some(arn) = topic.topic_arn() {
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() == 6 && parts[5] == topic_name {
                    let arn_string = arn.to_string();
                    let arn = ARN::parse(&arn_string).map_err(|_| {
                        tracing::debug!("ARN not parsable");
                        AlertError::TopicARNInvalid(arn_string)
                    })?;
                    return Ok(arn);
                }
            }
        }

        Err(AlertError::TopicNotFound(topic_name.to_string()))
    }

    pub fn is_valid_topic_name(&self, name: &str) -> bool {
        // AWS SNS topic name requirements:
        // - Can include numbers, letters, hyphens, and underscores
        // - Length between 1 and 256
        if name.is_empty() || name.len() > 256 {
            return false;
        }

        name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    }
}

pub struct SNS {
    inner: InnerAWSSNS,
    pub alert_identifier: AWSResourceIdentifier,
    cached_alert_arn: Arc<OnceLock<ARN>>,
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
    pub(crate) fn new(aws_config: &SdkConfig, args: &AlertArgs) -> Self {
        let latest_aws_config = match &args.alert_identifier {
            AWSResourceIdentifier::ARN(arn) => {
                // Extract region from ARN and create new config with that region
                if !arn.region.is_empty() {
                    aws_config.clone().into_builder().region(Region::new(arn.region.clone())).build()
                } else {
                    // If ARN has empty region, use provided config
                    aws_config.clone()
                }
            }
            AWSResourceIdentifier::Name(_) => {
                // Use provided config for name-based identifier
                aws_config.clone()
            }
        };

        Self {
            inner: InnerAWSSNS::new(&latest_aws_config),
            alert_identifier: args.alert_identifier.clone(),
            cached_alert_arn: Arc::new(OnceLock::new()),
        }
    }

    /// get_topic_arn return the topic name, if empty it will return an error
    ///
    /// # Returns
    ///
    /// * `Result<String, AlertError>` - The topic arn.
    pub async fn get_topic_arn(&self) -> Result<String, AlertError> {
        // Return cached value if available
        if let Some(arn) = self.cached_alert_arn.get() {
            return Ok(arn.to_string());
        }

        // Get ARN based on identifier type
        let arn = match &self.alert_identifier {
            AWSResourceIdentifier::ARN(arn) => arn.clone(),
            AWSResourceIdentifier::Name(name) => self.inner.fetch_topic_arn_by_name_via_client(name).await?,
        };

        // Cache and return
        let _ = self.cached_alert_arn.set(arn.clone());
        Ok(arn.to_string())
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
}
