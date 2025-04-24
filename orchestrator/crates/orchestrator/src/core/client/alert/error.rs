use thiserror::Error;

#[derive(Error, Debug)]
pub enum AlertError {
    #[error("Topic ARN is empty")]
    TopicARNEmpty,

    #[error("Unable to extract topic name")]
    UnableToExtractTopicName,

    #[error("Failed to send alert: {0}")]
    SendFailure(String),
}
