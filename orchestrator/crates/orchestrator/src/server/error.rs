use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::types::ApiResponse;

pub type ApiServiceError = JobRouteError;

pub type ApiServiceResult<T> = Result<T, JobRouteError>;

/// TODO: Make sure the response should be in json format and not plain text when throwing error

/// Represents errors that can occur during job route handling operations.
///
/// This enum implements both `Debug` and the custom `Error` trait from thiserror,
/// providing formatted error messages for each variant.
///
/// # Error Variants
/// Each variant maps to a specific HTTP status code when converted to a response:
/// * `InvalidId` - 400 Bad Request
/// * `NotFound` - 404 Not Found
/// * `ProcessingError` - 400 Bad Request
/// * `InvalidJobState` - 409 Conflict
/// * `DatabaseError` - 500 Internal Server Error
/// * `InvalidStatus` - 400 Bad Request
///
/// # Examples
/// ```
/// use orchestrator::server::error::JobRouteError;
///
/// // Creating an invalid ID error
/// let error = JobRouteError::InvalidId("123-invalid".to_string());
///
/// // Creating a processing error
/// let error = JobRouteError::ProcessingError("Failed to process job".to_string());
/// ```
#[derive(Debug, thiserror::Error)]
pub enum JobRouteError {
    /// Indicates that the provided job ID is not valid (e.g., not a valid UUID)
    #[error("Invalid job ID: {0}")]
    InvalidId(String),

    /// Indicates that the requested job could not be found in the system
    #[error("Job not found: {0}")]
    NotFound(String),

    /// Represents errors that occur during job processing
    #[error("Job processing error: {0}")]
    ProcessingError(String),

    /// Indicates that the job is in an invalid state for the requested operation
    #[error("Invalid job state: {0}")]
    InvalidJobState(String),

    /// Represents errors from database operations
    #[error("Database error")]
    DatabaseError,

    /// Indicates that the job status is invalid for the requested operation
    /// Contains both the job ID and the current status
    #[error("Invalid status: {id}: {job_status}")]
    InvalidStatus { id: String, job_status: String },
}

/// Implementation of axum's `IntoResponse` trait for converting errors into HTTP responses.
///
/// This implementation ensures that each error variant is mapped to an appropriate
/// HTTP status code and formatted response body.
///
/// # Response Format
/// All responses are returned as JSON with the following structure:
/// ```json
/// {
///     "success": false,
///     "message": "Error message here"
/// }
/// ```
///
/// # Status Code Mapping
/// * `InvalidId` -> 400 Bad Request
/// * `NotFound` -> 404 Not Found
/// * `ProcessingError` -> 400 Bad Request
/// * `InvalidJobState` -> 409 Conflict
/// * `DatabaseError` -> 500 Internal Server Error
/// * `InvalidStatus` -> 400 Bad Request
///
/// # Examples
/// This implementation is used automatically when returning errors from route handlers:
/// ```rust
/// use axum::response::Response;
///
///  async fn handle_job(id: String) -> Result<Response, JobRouteError> {
///     if !is_valid_id(&id) {
///         return Err(JobRouteError::InvalidId(id));
///     }
///     // ... rest of handler
/// }
/// ```
impl IntoResponse for JobRouteError {
    fn into_response(self) -> Response {
        match self {
            JobRouteError::InvalidId(id) => {
                (StatusCode::BAD_REQUEST, Json(ApiResponse::error(format!("Invalid job ID: {}", id)))).into_response()
            }
            JobRouteError::NotFound(id) => {
                (StatusCode::NOT_FOUND, Json(ApiResponse::error(format!("Job not found: {}", id)))).into_response()
            }
            JobRouteError::ProcessingError(msg) => {
                (StatusCode::BAD_REQUEST, Json(ApiResponse::error(format!("Processing error: {}", msg))))
                    .into_response()
            }
            JobRouteError::InvalidJobState(msg) => {
                (StatusCode::CONFLICT, Json(ApiResponse::error(format!("Invalid job state: {}", msg)))).into_response()
            }
            JobRouteError::DatabaseError => {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse::error("Database error occurred".to_string())))
                    .into_response()
            }
            JobRouteError::InvalidStatus { id, job_status } => (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::error(format!("Cannot retry job {id}: invalid status {job_status}"))),
            )
                .into_response(),
        }
    }
}
