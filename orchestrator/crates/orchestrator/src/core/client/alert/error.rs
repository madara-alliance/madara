use aws_sdk_sns::error::SdkError;
use aws_sdk_sns::operation::list_topics::ListTopicsError;
use aws_sdk_sns::operation::publish::PublishError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AlertError {
    #[error("Topic ARN is empty")]
    TopicARNEmpty,
    
    #[error("Topic ARN is invalid")]
    TopicARNInvalid,

    #[error("Unable to extract topic name : {0}")]
    UnableToExtractTopicName(String),

    #[error("Failed to send alert: {0}")]
    SendFailure(#[from] SdkError<PublishError>),

    #[error("Topic not found: {0}")]
    TopicNotFound(String),

    #[error("Failed to list alert topics: {0}")]
    ListTopicsError(#[from] SdkError<ListTopicsError>),

    #[error("Failed to take lock: {0}")]
    LockError(String),
}
