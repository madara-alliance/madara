//! HTTP transport utilities for Atlantic API
//!
//! This module provides HTTP transport utilities for the Atlantic API client:
//! authentication and error classification.
//!
//! # Components
//!
//! - [`ApiKeyAuth`]: Validates and stores API keys, providing header injection for authenticated requests.
//!
//! - [`HttpResponseClassifier`]: Classifies HTTP error responses as either infrastructure errors
//!   (retryable) or API errors (non-retryable). This enables smart retry logic that only
//!   retries transient failures.

use reqwest::header::{HeaderName, HeaderValue};
use reqwest::StatusCode;

use crate::error::AtlanticError;

/// Wraps API key and provides header injection for authenticated requests
///
/// This struct centralizes API key validation and header injection,
/// eliminating code duplication across multiple API methods.
///
/// # Example
/// ```ignore
/// let auth = ApiKeyAuth::new(&api_key)?;
/// let header_value = auth.header_value();
/// request.header(HeaderName::from_static("api-key"), header_value)
/// ```
#[derive(Clone, Debug)]
pub struct ApiKeyAuth {
    header_value: HeaderValue,
}

impl ApiKeyAuth {
    /// Creates a new API key authentication handler
    ///
    /// Validates the API key format at construction time.
    ///
    /// # Arguments
    /// * `api_key` - The API key string
    ///
    /// # Errors
    /// Returns an error if the API key is empty or contains invalid header characters
    pub fn new(api_key: impl AsRef<str>) -> Result<Self, AtlanticError> {
        let api_key = api_key.as_ref();

        if api_key.is_empty() {
            return Err(AtlanticError::Other {
                operation: "authentication".to_string(),
                message: "API key cannot be empty".to_string(),
            });
        }

        let header_value = HeaderValue::from_str(api_key).map_err(|e| AtlanticError::Other {
            operation: "authentication".to_string(),
            message: format!("Invalid API key format: {}", e),
        })?;

        Ok(Self { header_value })
    }

    /// Returns the header name for the API key
    #[inline]
    pub fn header_name() -> HeaderName {
        HeaderName::from_static("api-key")
    }

    /// Returns the header value for the API key
    #[inline]
    pub fn header_value(&self) -> HeaderValue {
        self.header_value.clone()
    }
}

/// Classifies HTTP responses for retry decisions
///
/// Distinguishes between infrastructure errors (load balancer, gateway)
/// which should be retried, and real API errors which should not.
pub struct HttpResponseClassifier;

impl HttpResponseClassifier {
    /// Determines if an HTTP error response indicates an infrastructure issue
    ///
    /// Infrastructure errors are transient and should be retried:
    /// - Load balancer errors (502, 503, 504)
    /// - Gateway timeouts
    /// - HTML error pages from nginx/apache
    /// - Malformed responses from proxies
    ///
    /// API errors are permanent and should not be retried:
    /// - 4xx errors with proper JSON error bodies
    /// - Business logic errors (bucket not found, invalid parameters)
    ///
    /// # Arguments
    /// * `status` - HTTP status code
    /// * `response_text` - Response body text
    ///
    /// # Returns
    /// `true` if this is an infrastructure error that should be retried
    pub fn is_infrastructure_error(status: StatusCode, response_text: &str) -> bool {
        // Common gateway/load balancer error pages
        if response_text.contains("404 page not found")
            || response_text.contains("502 Bad Gateway")
            || response_text.contains("503 Service Unavailable")
            || response_text.contains("504 Gateway Timeout")
            || response_text.contains("<html")
            || response_text.contains("<!DOCTYPE")
        {
            return true;
        }

        // Edge case: nginx sometimes returns a character-by-character JSON encoding
        // of error strings under high load. Observed during stress testing when nginx
        // buffers get corrupted. Example: {"0":"4","1":"0","2":"4",...} for "404 page not found"
        // This is a valid JSON object but NOT a valid Atlantic API response.
        if response_text.starts_with("{\"0\":") && response_text.contains("\"4\"") && response_text.contains("\"0\"") {
            return true;
        }

        // 502/503/504 are almost always infrastructure issues
        if status == StatusCode::BAD_GATEWAY
            || status == StatusCode::SERVICE_UNAVAILABLE
            || status == StatusCode::GATEWAY_TIMEOUT
        {
            return true;
        }

        // Special handling for 404 errors - need to distinguish infrastructure vs API errors
        // Infrastructure 404: nginx/load balancer couldn't route the request
        // API 404: Atlantic received the request but resource doesn't exist
        if status == StatusCode::NOT_FOUND {
            // Primary check: valid JSON indicates a real API response
            // Atlantic API always returns structured JSON for its errors
            if serde_json::from_str::<serde_json::Value>(response_text).is_ok() {
                // Valid JSON = real API response from Atlantic, not infrastructure error
                // This includes {"error": "not found"} style responses
                return false;
            }

            // Not valid JSON - likely an infrastructure error page (nginx, CDN, etc.)
            // Secondary check: look for API-specific keywords as a fallback
            // in case the response is plain text from the API (unlikely but defensive)
            let has_api_keywords = response_text.contains("bucket")
                || response_text.contains("query")
                || response_text.contains("job")
                || response_text.contains("error")
                || response_text.contains("message");

            // If no API keywords and not valid JSON, treat as infrastructure error
            !has_api_keywords
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod api_key_auth {
        use super::*;

        #[test]
        fn test_valid_api_key() {
            let auth = ApiKeyAuth::new("valid-api-key-123");
            assert!(auth.is_ok());
        }

        #[test]
        fn test_empty_api_key_rejected() {
            let auth = ApiKeyAuth::new("");
            assert!(auth.is_err());

            let err = auth.unwrap_err();
            match err {
                AtlanticError::Other { message, .. } => {
                    assert!(message.contains("empty"));
                }
                _ => panic!("Expected Other error"),
            }
        }

        #[test]
        fn test_header_name() {
            assert_eq!(ApiKeyAuth::header_name().as_str(), "api-key");
        }

        #[test]
        fn test_header_value() {
            let auth = ApiKeyAuth::new("test-key").unwrap();
            assert_eq!(auth.header_value().to_str().unwrap(), "test-key");
        }
    }

    mod http_response_classifier {
        use super::*;

        #[test]
        fn test_infrastructure_404_page_not_found() {
            assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::NOT_FOUND, "404 page not found"));
        }

        #[test]
        fn test_infrastructure_502_bad_gateway() {
            assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::BAD_GATEWAY, "502 Bad Gateway"));
        }

        #[test]
        fn test_infrastructure_503_service_unavailable() {
            assert!(HttpResponseClassifier::is_infrastructure_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "503 Service Unavailable"
            ));
        }

        #[test]
        fn test_infrastructure_504_gateway_timeout() {
            assert!(HttpResponseClassifier::is_infrastructure_error(
                StatusCode::GATEWAY_TIMEOUT,
                "504 Gateway Timeout"
            ));
        }

        #[test]
        fn test_infrastructure_weird_json_encoded_404() {
            // This weird format was observed during stress testing
            assert!(HttpResponseClassifier::is_infrastructure_error(
                StatusCode::NOT_FOUND,
                r#"{"0":"4","1":"0","2":"4"}"#
            ));
        }

        #[test]
        fn test_api_error_bucket_not_found() {
            // Real API 404 with valid JSON should NOT be classified as infrastructure error
            assert!(!HttpResponseClassifier::is_infrastructure_error(
                StatusCode::NOT_FOUND,
                r#"{"error": "bucket not found"}"#
            ));
        }

        #[test]
        fn test_api_error_valid_json_404() {
            // Any valid JSON 404 response should be treated as API error, not infrastructure
            assert!(!HttpResponseClassifier::is_infrastructure_error(
                StatusCode::NOT_FOUND,
                r#"{"status": "not_found", "code": 404}"#
            ));
        }

        #[test]
        fn test_infrastructure_invalid_json_404() {
            // 404 with invalid JSON and no API keywords = infrastructure error
            assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::NOT_FOUND, "Not Found"));
        }

        #[test]
        fn test_infrastructure_html_404() {
            // HTML 404 page = infrastructure error
            assert!(HttpResponseClassifier::is_infrastructure_error(
                StatusCode::NOT_FOUND,
                "<html><body>404 Not Found</body></html>"
            ));
        }
    }
}
