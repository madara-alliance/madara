use orchestrator_prover_client_interface::ProverClientError;
use reqwest::StatusCode;

use crate::transport::HttpResponseClassifier;

/// Errors that can occur when interacting with the Atlantic API.
///
/// # Retry Behavior
///
/// Only `NetworkError` is retryable. All other error types are considered permanent failures:
///
/// - **`NetworkError` (Retryable)**: Infrastructure-level errors like timeouts, incomplete messages,
///   connection issues, and gateway errors (502/503/504). These are transient and will be
///   automatically retried up to `RETRY_MAX_ATTEMPTS` times.
///
/// - **`ApiError` (Non-retryable)**: Real Atlantic API errors with proper HTTP responses.
///   This includes 4xx client errors and 5xx server errors that return valid JSON error messages.
///   These indicate application-level failures that won't be resolved by retrying.
///
/// - **Other error types (Non-retryable)**: `FileError`, `ParseError`, `UrlError`, and `Other`
///   represent programming errors or configuration issues that cannot be resolved by retrying.
///
/// ## Important Note on 5xx Errors
///
/// Real Atlantic API 5xx errors with valid JSON responses are treated as `ApiError` and will
/// **NOT** be retried. Only infrastructure-level 5xx errors (like gateway timeouts or HTML
/// error pages from load balancers) are classified as `NetworkError` and retried.
#[derive(Debug, thiserror::Error)]
pub enum AtlanticError {
    /// Network/transport errors that may be retryable (timeouts, incomplete messages, etc.)
    #[error("Network error during {operation}: {message}")]
    NetworkError { operation: String, message: String, response_bytes: Option<u64> },

    /// Atlantic API returned an error response (4xx/5xx status codes)
    #[error("Atlantic API error during {operation} (status {status}): {message}")]
    ApiError { operation: String, status: StatusCode, message: String, response_bytes: Option<u64> },

    /// File system errors
    #[error("File error during {operation}: {message}")]
    FileError { operation: String, message: String },

    /// JSON parsing errors
    #[error("Failed to parse response during {operation}: {message}")]
    ParseError { operation: String, message: String },

    /// URL/path segment errors
    #[error("Failed to build URL for {operation}: {message}")]
    UrlError { operation: String, message: String },

    /// Other unexpected errors
    #[error("Unexpected error during {operation}: {message}")]
    Other { operation: String, message: String },
}

impl AtlanticError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, AtlanticError::NetworkError { .. })
    }

    /// Get error type as a string for metrics
    ///
    /// Returns a static string classification of the error type for use in OpenTelemetry metrics.
    /// This allows error types to be used as metric labels for monitoring and alerting.
    ///
    /// # Returns
    /// A static string that will never be empty, ensuring metrics always have a valid error_type label:
    /// - "network_error" - Infrastructure/transport errors
    /// - "api_error" - Atlantic API errors
    /// - "file_error" - File system errors
    /// - "parse_error" - JSON parsing errors
    /// - "url_error" - URL construction errors
    /// - "other_error" - Unexpected errors
    pub fn error_type(&self) -> &'static str {
        match self {
            AtlanticError::NetworkError { .. } => "network_error",
            AtlanticError::ApiError { .. } => "api_error",
            AtlanticError::FileError { .. } => "file_error",
            AtlanticError::ParseError { .. } => "parse_error",
            AtlanticError::UrlError { .. } => "url_error",
            AtlanticError::Other { .. } => "other_error",
        }
    }

    /// Create a network error from a reqwest error
    pub fn from_reqwest_error(operation: impl Into<String>, source: reqwest::Error) -> Self {
        let operation = operation.into();

        // Check if it's a network-level error (retryable)
        if source.is_timeout() || source.is_connect() || source.is_request() {
            let message = if source.is_timeout() {
                "request timed out".to_string()
            } else if source.is_connect() {
                format!("connection failed: {}", source)
            } else {
                // Check for specific hyper errors like IncompleteMessage
                let error_msg = source.to_string();
                if error_msg.contains("IncompleteMessage") {
                    "incomplete message received from server".to_string()
                } else if error_msg.contains("Canceled") {
                    "request was canceled".to_string()
                } else {
                    format!("request failed: {}", error_msg)
                }
            };

            AtlanticError::NetworkError { operation, message, response_bytes: None }
        } else if let Some(status) = source.status() {
            // HTTP error with status code (non-retryable)
            AtlanticError::ApiError { operation, status, message: source.to_string(), response_bytes: None }
        } else {
            // Other reqwest errors
            AtlanticError::NetworkError { operation, message: source.to_string(), response_bytes: None }
        }
    }

    /// Create a file error from an io error
    pub fn from_io_error(operation: impl Into<String>, source: std::io::Error) -> Self {
        AtlanticError::FileError { operation: operation.into(), message: source.to_string() }
    }

    /// Create a parse error
    pub fn parse_error(operation: impl Into<String>, message: impl Into<String>) -> Self {
        AtlanticError::ParseError { operation: operation.into(), message: message.into() }
    }

    /// Create an API error with custom message
    pub fn api_error(operation: impl Into<String>, status: StatusCode, message: impl Into<String>) -> Self {
        AtlanticError::ApiError { operation: operation.into(), status, message: message.into(), response_bytes: None }
    }

    /// Create an error from HTTP response, detecting infrastructure errors
    ///
    /// Infrastructure errors (load balancer/gateway errors) are treated as retryable NetworkErrors,
    /// while real Atlantic API errors are treated as non-retryable ApiErrors.
    ///
    /// Uses `HttpResponseClassifier` from the transport layer to determine if the error
    /// is an infrastructure issue (retryable) or a real API error (non-retryable).
    ///
    /// # Arguments
    /// * `operation` - The operation name (e.g., "add_job")
    /// * `status` - HTTP status code
    /// * `response_text` - Response body text
    ///
    /// # Returns
    /// * `NetworkError` if this is an infrastructure/gateway error (retryable)
    /// * `ApiError` if this is a real Atlantic API error (non-retryable)
    pub fn from_http_error_response(
        operation: impl Into<String>,
        status: StatusCode,
        response_text: impl AsRef<str>,
    ) -> Self {
        let operation = operation.into();
        let text = response_text.as_ref();
        let response_bytes = Some(text.len() as u64);

        if HttpResponseClassifier::is_infrastructure_error(status, text) {
            // Treat as network error (retryable)
            AtlanticError::NetworkError {
                operation,
                message: format!("infrastructure error (status {}): {}", status, text),
                response_bytes,
            }
        } else {
            // Real API error (non-retryable)
            AtlanticError::ApiError { operation, status, message: text.to_string(), response_bytes }
        }
    }
}

impl From<AtlanticError> for ProverClientError {
    fn from(value: AtlanticError) -> Self {
        Self::Internal(Box::new(value))
    }
}
