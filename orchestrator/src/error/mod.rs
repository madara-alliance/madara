pub mod consumer;
pub mod event;
pub mod job;
pub mod other;

use alloy::hex::FromHexError;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::list_buckets::ListBucketsError;
use aws_sdk_sqs::operation::set_queue_attributes::SetQueueAttributesError;
use mongodb::bson;
use opentelemetry_otlp::ExporterBuildError;
use opentelemetry_sdk::trace::TraceError;
use thiserror::Error;

use crate::core::client::alert::AlertError;
use crate::core::client::database::DatabaseError;
use crate::core::client::queue::QueueError;
use crate::core::client::storage::StorageError;
use crate::core::error::OrchestratorCoreError;
use crate::error::event::EventSystemError;
pub use consumer::ConsumptionError;

/// Result type for orchestrator operations
pub type OrchestratorResult<T> = Result<T, OrchestratorError>;

/// Error types for the orchestrator
#[derive(Error, Debug)]
pub enum OrchestratorError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),

    #[error("Alert error: {0}")]
    AlertError(#[from] AlertError),

    #[error("Queue error: {0}")]
    QueueCoreError(#[from] QueueError),

    #[error("Database error: {0}")]
    DatabaseCoreError(#[from] DatabaseError),

    #[error("Orchestrator Core Error: {0}")]
    OrchestratorCoreError(#[from] OrchestratorCoreError),

    /// Setup Command error
    #[error("Setup Command Error: {0}")]
    SetupCommandError(String),

    /// Run Command error
    #[error("Run Command Error: {0}")]
    RunCommandError(String),

    /// Setup Command error
    #[error("Error While Downcasting from object: {0}")]
    FromDownstreamError(String),

    #[error("Orchestrator Error: {0}")]
    OrchestratorAnyHowError(#[from] anyhow::Error),

    /// Error while instrumenting the logger
    #[error("OTL Logger Error: {0}")]
    OTLogError(#[from] opentelemetry_sdk::error::OTelSdkError),
    #[error("OLT Exporter Build Error: {0}")]
    ExporterBuildError(#[from] ExporterBuildError),
    #[error("OLT Metrics Error: {0}")]
    OTLMetricsError(String),
    #[error("OLT Trace Error: {0}")]
    OLTTraceError(#[from] TraceError),
    #[error("Invalid layout name: {0}")]
    InvalidLayoutError(String),
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
    /// Setup error
    #[error("Setup error: {0}")]
    SetupError(String),
    /// AWS SDK error
    #[error("AWS SDK error: {0}")]
    AWSSDKError(#[from] SdkError<ListBucketsError>),
    /// AWS SQS error
    #[error("AWS SQS error: {0}")]
    AWSSQSError(#[from] SdkError<SetQueueAttributesError>),
    /// AWS S3 error
    #[error("AWS S3 error: {0}")]
    AWSS3Error(#[from] SdkError<GetObjectError>),
    #[error("AWS S3 error: {0}")]
    AWSS3StreamError(String),
    /// Unknown Resource error
    #[error("Resource provided is not defined: {0}")]
    UnidentifiedResourceError(String),
    #[error("Queue error: {0}")]
    QueueError(#[from] omniqueue::QueueError),
    #[error("ConsumptionError: {0}")]
    ConsumptionError(#[from] ConsumptionError),

    #[error("Unknow Resource type: {0}")]
    UnknownResourceTypeError(String),

    /// Database error
    #[error("Database Invalid URI error: {0}")]
    DatabaseInvalidURIError(String),

    /// Database error
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Mongo error
    #[error("Mongo error: {0}")]
    MongoError(#[from] mongodb::error::Error),

    /// Mongo error
    #[error("BSON error: {0}")]
    BsonError(#[from] bson::ser::Error),

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

    /// From Hex error
    #[error("Invalid address: {0}")]
    InvalidAddress(#[from] FromHexError),

    #[error("Event System Error: {0}")]
    EventSystemError(#[from] EventSystemError),
}
