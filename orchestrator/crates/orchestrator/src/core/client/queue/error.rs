use aws_sdk_sqs::error::SdkError;
use aws_sdk_sqs::operation::get_queue_attributes::GetQueueAttributesError;
use aws_sdk_sqs::operation::get_queue_url::GetQueueUrlError;
use omniqueue::QueueError as OmniQueueError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum QueueError {
    #[error("Missing parameter: {0}")]
    MissingRootParameter(String),

    #[error("Failed to get queue url: {0}")]
    GetQueueUrlError(#[from] SdkError<GetQueueUrlError>),

    #[error("Failed to get queue attributes: {0}")]
    GetQueueAttributesError(#[from] SdkError<GetQueueAttributesError>),

    #[error("Failed to get queue url: {0}")]
    ErrorFromQueueError(#[from] OmniQueueError),

    #[error("Failed to get queue url for queue name : {0}")]
    FailedToGetQueueUrl(String),
}
