use super::AlertError;
use crate::{core::client::alert::AlertClient, types::params::AlertArgs};
use async_trait::async_trait;
use aws_config::Region;
use aws_config::SdkConfig;
use aws_sdk_sns::Client;
use std::sync::{Arc, OnceLock};

pub struct SNS {
    pub client: Arc<Client>,
    // We need to keep these as Options since setup's create_setup fns passes args as None.
    topic_name: Option<String>,       // The topic name
    region: Option<String>,           // Region extracted from ARN (if provided)
    account_id: Option<String>,       // Account ID extracted from ARN (if provided)
    topic_arn: Arc<OnceLock<String>>, // Cached resolved ARN
}

impl SNS {
    /// Create a new SNS client with options for cross-account access
    ///
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    /// * `args` - The alert arguments containing topic_identifier (name or ARN).
    ///
    /// # Returns
    /// * `Self` - The SNS client.
    pub(crate) fn new(aws_config: &SdkConfig, args: Option<&AlertArgs>) -> Self {
        let (topic_name, region, account_id) = if let Some(args) = args {
            // Parse the queue identifier to handle both ARN and template formats
            Self::parse_arn_topic_identifier(args)
        } else {
            (None, None, None)
        };

        // Configure SNS client with the right region if specified in ARN
        let mut sns_config_builder = aws_sdk_sns::config::Builder::from(aws_config);

        // Set region from ARN if available
        if let Some(region_str) = &region {
            sns_config_builder = sns_config_builder.region(Region::new(region_str.clone()));
        }

        let client = Client::from_conf(sns_config_builder.build());

        Self { client: Arc::new(client), topic_name, region, account_id, topic_arn: Arc::new(OnceLock::new()) }
    }

    /// Parse a topic identifier (name or ARN) to extract components
    /// Returns: (topic_identifier, region, account_id)
    fn parse_arn_topic_identifier(args: &AlertArgs) -> (Option<String>, Option<String>, Option<String>) {
        let identifier = &args.topic_identifier;

        // Check if the identifier is an SNS ARN
        if identifier.starts_with("arn:aws:sns:") {
            let parts: Vec<&str> = identifier.split(':').collect();

            // Standard SNS ARN has format arn:aws:sns:{region}:{account-id}:{topic-name}
            // // Note: We don't add AWS_PREFIX if we received the whole ARN !
            if parts.len() == 6 {
                let region = parts[3].to_string();
                let account_id = parts[4].to_string();

                return (Some(parts[5].to_string()), Some(region), Some(account_id));
            }
        }

        // If not an ARN, just use as a topic name with prefix
        (Some(format!("{}_{}", args.aws_prefix, identifier)), None, None)
    }

    /// Constructs an ARN from components if available, or returns the original identifier
    fn construct_arn_if_possible(&self) -> Option<String> {
        if let (Some(topic_name), Some(region), Some(account_id)) = (&self.topic_name, &self.region, &self.account_id) {
            Some(format!("arn:aws:sns:{}:{}:{}", region, account_id, topic_name))
        } else {
            None
        }
    }

    /// get_topic_arn resolves the topic ARN, either from cached value,
    /// by constructing it from ARN components, or by looking it up
    ///
    /// # Returns
    /// * `Result<String, AlertError>` - The topic ARN.
    pub async fn get_topic_arn(&self) -> Result<String, AlertError> {
        // First, try to get the cached value
        if let Some(arn) = self.topic_arn.get() {
            return Ok(arn.clone());
        }

        // If we have topic_name, region and account_id, we can construct the ARN directly
        if let Some(arn) = self.construct_arn_if_possible() {
            // Cache the ARN
            let _ = self.topic_arn.set(arn.clone());
            return Ok(arn);
        }

        // Check if we have a topic name to look up
        let topic_name = self.topic_name.as_ref().ok_or(AlertError::TopicNameEmpty)?;

        // Look up the ARN by listing topics and matching the name
        let resp = self.client.list_topics().send().await.map_err(AlertError::ListTopicsError)?;

        for topic in resp.topics() {
            if let Some(arn) = topic.topic_arn() {
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() == 6 && parts[5] == topic_name {
                    let arn_string = arn.to_string();
                    let _ = self.topic_arn.set(arn_string.clone());
                    return Ok(arn_string);
                }
            }
        }

        // If we got here, the topic wasn't found
        Err(AlertError::TopicNotFound(topic_name.clone()))
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
        let arn = self.get_topic_arn().await?;

        Ok(arn.clone().split(':').last().ok_or(AlertError::UnableToExtractTopicName(arn))?.to_string())
    }
}
