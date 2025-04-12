use thiserror::Error;

#[derive(Error, Debug)]
pub enum CronError {
    #[error("Failed to add cron target queue: {0}")]
    FailedToAddCronTargetQueue(String),
}
