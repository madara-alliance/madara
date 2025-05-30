use thiserror::Error;

#[derive(Error, Debug)]
pub enum EventBusError {
    #[error("Failed to add cron target queue: {0}")]
    FailedToAddCronTargetQueue(String),

    #[error("Invalid ARN: {0}")]
    InvalidArn(String),
}
