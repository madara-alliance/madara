use aws_sdk_sns::error::SdkError;
use aws_sdk_sns::operation::publish::PublishError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AlertError {
    #[error("Topic ARN is empty")]
    TopicARNEmpty,

    #[error("Unable to extract topic name : {0}")]
    UnableToExtractTopicName(String),

    #[error("Failed to send alert: {0}")]
    SendFailure(#[from] SdkError<PublishError>),
}
