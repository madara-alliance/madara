use alloy::primitives::hex::FromHexError;
use orchestrator_prover_client_interface::ProverClientError;
use reqwest::StatusCode;

type StatusMessage = String;

#[derive(Debug, thiserror::Error)]
pub enum AtlanticError {
    #[error("Failed to add Atlantic job: {0}")]
    AddJobFailure(#[source] reqwest::Error),

    #[error("Failed to get status of an Atlantic job: {0}")]
    GetJobStatusFailure(#[source] reqwest::Error),

    #[error("Failed to get artifacts of an Atlantic job: {0}")]
    GetJobArtifactsFailure(#[source] reqwest::Error),

    #[error("Failed to submit L2 query: {0}")]
    SubmitL2QueryFailure(#[source] reqwest::Error),

    #[error("Atlantic service returned an error {0}")]
    AtlanticService(StatusCode, StatusMessage),

    #[error("Failed to read file: {0}")]
    FileError(#[from] std::io::Error),

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

    #[error("Failed to open file: {0}")]
    FileOpenError(#[source] tokio::io::Error),

    #[error("Failed to create mime string: {0}")]
    MimeError(#[source] reqwest::Error),

    #[error("Failed to read file: {0}")]
    FileReadError(#[source] tokio::io::Error),

    #[error("Other error: {0}")]
    Other(#[from] color_eyre::eyre::Error),

    #[error("Failed to parse body: {0}")]
    BodyParseError(#[source] serde_json::Error),

    #[error("Failed to get status of an Atlantic bucket: {0}")]
    GetBucketStatusFailure(#[source] reqwest::Error),

    #[error("Failed to create a new Atlantic bucket: {0}")]
    CreateBucketFailure(#[source] reqwest::Error),

    #[error("Failed to close Atlantic bucket: {0}")]
    CloseBucketFailure(#[source] reqwest::Error),
}

impl From<AtlanticError> for ProverClientError {
    fn from(value: AtlanticError) -> Self {
        Self::Internal(Box::new(value))
    }
}
