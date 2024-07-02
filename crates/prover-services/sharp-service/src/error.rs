use alloy::primitives::hex::FromHexError;
use gps_fact_checker::error::FactCheckerError;
use prover_client_interface::ProverClientError;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum SharpError {
    #[error("Failed to to add SHARP job: {0}")]
    AddJobFailure(#[source] reqwest::Error),
    #[error("Failed to to get status of a SHARP job: {0}")]
    GetJobStatusFailure(#[source] reqwest::Error),
    #[error("Fact checker error: {0}")]
    FactChecker(#[from] FactCheckerError),
    #[error("SHARP service returned an error {0}")]
    SharpService(StatusCode),
    #[error("Failed to parse job key: {0}")]
    JobKeyParse(uuid::Error),
    #[error("Failed to parse fact: {0}")]
    FactParse(FromHexError),
    #[error("Failed to split task id into job key and fact")]
    TaskIdSplit,
    #[error("Failed to encode PIE")]
    PieEncode(#[source] snos::error::SnOsError),
}

impl From<SharpError> for ProverClientError {
    fn from(value: SharpError) -> Self {
        Self::Internal(Box::new(value))
    }
}
