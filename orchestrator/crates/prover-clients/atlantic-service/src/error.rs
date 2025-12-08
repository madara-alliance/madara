use orchestrator_prover_client_interface::ProverClientError;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum AtlanticError {
    /// Network/transport errors that may be retryable (timeouts, incomplete messages, etc.)
    #[error("Network error during {operation}: {message}")]
    NetworkError { operation: String, message: String },

    /// Atlantic API returned an error response (4xx/5xx status codes)
    #[error("Atlantic API error during {operation} (status {status}): {message}")]
    ApiError { operation: String, status: StatusCode, message: String },

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

            AtlanticError::NetworkError { operation, message }
        } else if let Some(status) = source.status() {
            // HTTP error with status code (non-retryable)
            AtlanticError::ApiError { operation, status, message: source.to_string() }
        } else {
            // Other reqwest errors
            AtlanticError::NetworkError { operation, message: source.to_string() }
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
        AtlanticError::ApiError { operation: operation.into(), status, message: message.into() }
    }

    /// Create an error from HTTP response, detecting infrastructure errors
    ///
    /// Infrastructure errors (load balancer/gateway errors) are treated as retryable NetworkErrors,
    /// while real Atlantic API errors are treated as non-retryable ApiErrors.
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

        // Detect infrastructure/gateway errors that should be retried
        let is_infrastructure_error =
            // Common gateway/load balancer error pages
            text.contains("404 page not found") ||
            text.contains("502 Bad Gateway") ||
            text.contains("503 Service Unavailable") ||
            text.contains("504 Gateway Timeout") ||
            text.contains("<html") || // HTML error pages from nginx/apache
            text.contains("<!DOCTYPE") ||

            // Weird JSON-encoded string format (e.g., {"0":"4","1":"0",...} encoding "404 page not found")
            // This is what we saw in the logs - each character as a separate JSON key
            (text.starts_with("{\"0\":") && text.contains("\"4\"") && text.contains("\"0\"")) ||

            // 404 from infrastructure (not a real Atlantic API 404)
            // Real Atlantic 404s would have proper JSON with "error" field or mention specific entities
            (status == StatusCode::NOT_FOUND &&
                !text.contains("bucket") &&
                !text.contains("query") &&
                !text.contains("job") &&
                !text.contains("error") &&
                !text.contains("message")) ||

            // 502/503/504 are almost always infrastructure issues
            status == StatusCode::BAD_GATEWAY ||
            status == StatusCode::SERVICE_UNAVAILABLE ||
            status == StatusCode::GATEWAY_TIMEOUT;

        if is_infrastructure_error {
            // Treat as network error (retryable)
            AtlanticError::NetworkError {
                operation,
                message: format!("infrastructure error (status {}): {}", status, text),
            }
        } else {
            // Real API error (non-retryable)
            AtlanticError::ApiError { operation, status, message: text.to_string() }
        }
    }
}

impl From<AtlanticError> for ProverClientError {
    fn from(value: AtlanticError) -> Self {
        Self::Internal(Box::new(value))
    }
}
