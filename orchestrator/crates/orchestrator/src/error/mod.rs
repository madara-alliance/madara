use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::list_buckets::ListBucketsError;
use aws_sdk_sqs::operation::set_queue_attributes::SetQueueAttributesError;
use thiserror::Error;

/// Result type for orchestrator operations
pub type OrchestratorResult<T> = std::result::Result<T, OrchestratorError>;

/// Alias for OrchestratorResult for easier usage
pub type Result<T> = OrchestratorResult<T>;

/// Error types for the orchestrator
#[derive(Error, Debug)]
pub enum OrchestratorError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
    /// Cloud provider error
    #[error("Invalid Cloud Provider error: {0}")]
    InvalidCloudProviderError(String),
    /// Invalid region error
    #[error("Invalid cloud region error")]
    InvalidRegionError,
    /// Resource Already Exists error
    #[error("Resource already exists error: {0}")]
    ResourceAlreadyExistsError(String),
    /// Resource Setup error
    #[error("Resource setup error: {0}")]
    ResourceSetupError(String),
    /// AWS SDK error
    #[error("AWS SDK error: {0}")]
    AWSSDKError(#[from] SdkError<ListBucketsError>),
    /// AWS SQS error
    #[error("AWS SQS error: {0}")]
    AWSSQSError(#[from] SdkError<SetQueueAttributesError>),

    /// Unknown Resource error
    #[error("Resource provided is not defined: {0}")]
    UnidentifiedResourceError(String),

    /// Database error
    #[error("Database Invalid URI error: {0}")]
    DatabaseInvalidURIError(String),

    /// Database error
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Resource error
    #[error("Resource error: {0}")]
    ResourceError(String),

    /// Controller error
    #[error("Controller error: {0}")]
    ControllerError(String),

    /// Worker error
    #[error("Worker error: {0}")]
    WorkerError(String),

    /// Service error
    #[error("Service error: {0}")]
    ServiceError(String),

    /// Client error
    #[error("Client error: {0}")]
    ClientError(String),

    /// AWS error
    #[error("AWS error: {0}")]
    AwsError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// JSON error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}
