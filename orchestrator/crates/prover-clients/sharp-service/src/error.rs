use orchestrator_prover_client_interface::http::HttpResponseClassifier;
use orchestrator_prover_client_interface::retry::RetryableRequestError;
use orchestrator_prover_client_interface::ProverClientError;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum SharpError {
    #[error("Failed to add SHARP job: {0}")]
    AddJobFailure(#[source] reqwest::Error),

    #[error("Failed to add SHARP applicative job: {0}")]
    AddApplicativeJobFailure(#[source] reqwest::Error),

    #[error("Failed to get status of a SHARP job: {0}")]
    GetJobStatusFailure(#[source] reqwest::Error),

    #[error("Failed to get proof of a SHARP job: {0}")]
    GetProofFailure(#[source] reqwest::Error),

    #[error("SHARP service returned {status} for {url}")]
    SharpService { status: StatusCode, url: String, body: String },

    #[error("Failed to serialize body")]
    SerializationError(#[source] std::io::Error),

    #[error("Other error: {0}")]
    Other(#[from] color_eyre::eyre::Error),
}

impl RetryableRequestError for SharpError {
    fn is_retryable(&self) -> bool {
        match self {
            // Network-level transient failures (timeout, connection refused, DNS, etc.)
            SharpError::AddJobFailure(e)
            | SharpError::AddApplicativeJobFailure(e)
            | SharpError::GetJobStatusFailure(e)
            | SharpError::GetProofFailure(e) => e.is_timeout() || e.is_connect() || e.is_request(),
            // 5xx is always retryable. For other status codes, inspect the body to
            // distinguish infrastructure errors (LB/gateway HTML pages, nginx 404s)
            // from real SHARP API errors.
            SharpError::SharpService { status, body, .. } => {
                status.is_server_error() || HttpResponseClassifier::is_infrastructure_error(*status, body)
            }
            // Serialization + catch-all: non-retryable.
            SharpError::SerializationError(_) | SharpError::Other(_) => false,
        }
    }

    fn error_type(&self) -> &'static str {
        match self {
            SharpError::AddJobFailure(_) => "add_job_failure",
            SharpError::AddApplicativeJobFailure(_) => "add_applicative_job_failure",
            SharpError::GetJobStatusFailure(_) => "get_job_status_failure",
            SharpError::GetProofFailure(_) => "get_proof_failure",
            SharpError::SharpService { .. } => "sharp_service_error",
            SharpError::SerializationError(_) => "serialization_error",
            SharpError::Other(_) => "other",
        }
    }
}

impl From<SharpError> for ProverClientError {
    fn from(value: SharpError) -> Self {
        Self::Internal(Box::new(value))
    }
}
