use crate::types::jobs::types::{JobStatus, JobType};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::{BlockRouteError, JobRouteError};

/// Represents a job identifier in API requests.
///
/// This struct is used to deserialize job IDs from incoming HTTP requests,
/// particularly in path parameters.
///
/// # Examples
/// ```
/// use orchestrator::server::types::JobId;
/// let job_id = JobId { id: "123e4567-e89b-12d3-a456-426614174000".to_string() };
/// ```
#[derive(Deserialize)]
pub struct JobId {
    /// The string representation of the job's UUID
    pub id: String,
}

/// Represents a standardized API response structure.
///
/// This struct provides a consistent format for all API responses, including
/// both successful operations and errors. It implements serialization for
/// converting responses to JSON.
///
/// # Fields
/// * `success` - Indicates whether the operation was successful
/// * `message` - Optional message providing additional details (typically used for errors)
///
/// # Examples
/// ```
/// // Success response
/// use orchestrator::server::types::ApiResponse;
/// let response: ApiResponse<()> = ApiResponse::success(None);
/// assert_eq!(response.success, true);
/// assert_eq!(response.message, None);
///
/// // Error response
/// let response = ApiResponse::error("Invalid job ID".to_string());
/// assert_eq!(response.success, false);
/// assert_eq!(response.message, Some("Invalid job ID".to_string()));
/// ```
#[derive(Serialize, Deserialize)]
pub struct ApiResponse<T = ()> {
    /// Indicates if the operation was successful
    pub success: bool,
    /// Optional data payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Optional message, typically used for error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ApiResponse<()> {
    /// Creates an error response with the specified message.
    ///
    /// # Arguments
    /// * `message` - The error message to include in the response
    ///
    /// # Returns
    /// Returns an `ApiResponse` with `success` set to `false` and the provided message.
    ///
    /// # Examples
    /// ```
    /// use orchestrator::server::types::ApiResponse;
    /// let response = ApiResponse::error("Operation failed".to_string());
    /// assert_eq!(response.success, false);
    /// assert_eq!(response.message, Some("Operation failed".to_string()));
    /// ```
    pub fn error(message: String) -> Self {
        Self { success: false, data: None, message: Some(message) }
    }
}

impl<T> ApiResponse<T> {
    /// Creates a successful response with optional data and message.
    pub fn success_with_data(data: T, message: Option<String>) -> Self {
        Self { success: true, data: Some(data), message }
    }

    /// Creates a successful response with no message.
    ///
    /// # Returns
    /// Returns an `ApiResponse` with `success` set to `true` and no message.
    ///
    /// # Examples
    /// ```
    /// use orchestrator::server::types::ApiResponse;
    /// let response: ApiResponse<()> = ApiResponse::success(None);
    /// assert_eq!(response.success, true);
    /// ```
    pub fn success(message: Option<String>) -> Self {
        Self { success: true, data: None, message }
    }
}

/// Type alias for the result type used in job route handlers.
///
/// This type combines axum's `Response` type with our custom `JobRouteError`,
/// providing a consistent error handling pattern across all job-related routes.
///
/// # Examples
/// ```
/// use axum::Json;
/// use axum::response::IntoResponse;
/// use orchestrator::server::types::{ApiResponse, JobRouteResult};
/// use orchestrator::server::error::JobRouteError;
///
/// async fn handle_job() -> JobRouteResult {
///     // Success case
///     Ok(Json(ApiResponse::<()>::success(None)).into_response())
///     // Error case would be:
///     // Err(JobRouteError::NotFound("123".to_string()))
/// }
/// ```
pub type JobRouteResult = Result<Response<axum::body::Body>, JobRouteError>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobStatusResponseItem {
    pub job_type: JobType,
    pub id: Uuid,
    pub status: JobStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobStatusResponse {
    pub jobs: Vec<JobStatusResponseItem>,
}


pub type BlockRouteResult = Result<Response<axum::body::Body>, BlockRouteError>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockStatusResponse {
    pub batch_number: u64,
}
