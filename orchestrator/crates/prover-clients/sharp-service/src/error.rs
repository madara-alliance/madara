use alloy::primitives::hex::FromHexError;
use orchestrator_prover_client_interface::ProverClientError;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum SharpError {
    #[error("Failed to to add SHARP job: {0}")]
    AddJobFailure(#[source] reqwest::Error),

    #[error("Failed to to close SHARP bucket: {0}")]
    CloseBucketFailure(#[source] reqwest::Error),

    #[error("Failed to to create SHARP bucket: {0}")]
    CreateBucketFailure(#[source] reqwest::Error),

    #[error("Failed to to get artifacts of a SHARP job: {0}")]
    GetJobArtifactsFailure(#[source] reqwest::Error),

    #[error("Failed to to get status of a SHARP job: {0}")]
    GetJobStatusFailure(#[source] reqwest::Error),

    #[error("SHARP service returned an error {0}")]
    SharpService(StatusCode),

    #[error("Failed to parse job key: {0}")]
    JobKeyParse(uuid::Error),

    #[error("Failed to parse fact: {0}")]
    FactParse(FromHexError),

    #[error("Failed to split task id into job key and fact")]
    TaskIdSplit,

    #[error("Failed to encode PIE")]
    PieEncode(#[source] starknet_os::error::SnOsError),

    #[error("Failed to get url as path segment mut. URL is cannot-be-a-base.")]
    PathSegmentMutFailOnUrl,

    #[error("Failed to serialize body")]
    SerializationError(#[source] std::io::Error),

    #[error("Other error: {0}")]
    Other(#[from] color_eyre::eyre::Error),
}

impl From<SharpError> for ProverClientError {
    fn from(value: SharpError) -> Self {
        Self::Internal(Box::new(value))
    }
}
