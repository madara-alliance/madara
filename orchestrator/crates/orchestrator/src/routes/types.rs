use axum::response::Response;
use serde::{Deserialize, Serialize};

use super::error::JobRouteError;

/// Represents a job identifier in API requests.
///
/// This struct is used to deserialize job IDs from incoming HTTP requests,
/// particularly in path parameters.
///
/// # Examples
/// ```
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
/// let response = ApiResponse::success();
/// assert_eq!(response.success, true);
/// assert_eq!(response.message, None);
///
/// // Error response
/// let response = ApiResponse::error("Invalid job ID".to_string());
/// assert_eq!(response.success, false);
/// assert_eq!(response.message, Some("Invalid job ID".to_string()));
/// ```
#[derive(Serialize, Deserialize)]
pub struct ApiResponse {
    /// Indicates if the operation was successful
    pub success: bool,
    /// Optional message, typically used for error details
    pub message: Option<String>,
}

impl ApiResponse {
    /// Creates a successful response with no message.
    ///
    /// # Returns
    /// Returns an `ApiResponse` with `success` set to `true` and no message.
    ///
    /// # Examples
    /// ```
    /// let response = ApiResponse::success();
    /// assert_eq!(response.success, true);
    /// ```
    pub fn success(message: Option<String>) -> Self {
        Self { success: true, message }
    }

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
    /// let response = ApiResponse::error("Operation failed".to_string());
    /// assert_eq!(response.success, false);
    /// assert_eq!(response.message, Some("Operation failed".to_string()));
    /// ```
    pub fn error(message: String) -> Self {
        Self { success: false, message: Some(message) }
    }
}

/// Type alias for the result type used in job route handlers.
///
/// This type combines axum's `Response` type with our custom `JobRouteError`,
/// providing a consistent error handling pattern across all job-related routes.
///
/// # Examples
/// ```
/// async fn handle_job() -> JobRouteResult {
///     // Success case
///     Ok(Json(ApiResponse::success()).into_response())
///     
///     // Error case
///     Err(JobRouteError::NotFound("123".to_string()))
/// }
/// ```
pub type JobRouteResult = Result<Response, JobRouteError>;
