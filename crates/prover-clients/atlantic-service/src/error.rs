use alloy::primitives::hex::FromHexError;
use prover_client_interface::ProverClientError;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum AtlanticError {
    #[error("Failed to to add Atlantic job: {0}")]
    AddJobFailure(#[source] reqwest::Error),
    #[error("Failed to to get status of a Atlantic job: {0}")]
    GetJobStatusFailure(#[source] reqwest::Error),
    #[error("Atlantic service returned an error {0}")]
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
    #[error("Failed to open file: {0}")]
    FileOpenError(#[source] tokio::io::Error),
    #[error("Failed to create mime string: {0}")]
    MimeError(#[source] reqwest::Error),
    #[error("Failed to read file: {0}")]
    FileReadError(#[source] tokio::io::Error),
    #[error("Other error: {0}")]
    Other(#[from] color_eyre::eyre::Error),
}

impl From<AtlanticError> for ProverClientError {
    fn from(value: AtlanticError) -> Self {
        Self::Internal(Box::new(value))
    }
}
