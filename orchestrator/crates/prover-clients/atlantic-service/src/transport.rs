//! HTTP transport layer for Atlantic API
//!
//! This module provides low-level HTTP transport abstractions for the Atlantic API client.
//! It handles authentication and HTTP error classification, separating these concerns
//! from business logic and API-specific request/response handling.
//!
//! # Components
//!
//! - [`ApiKeyAuth`]: Validates and stores API keys, providing header injection for authenticated requests.
//!   Centralizes API key handling to eliminate code duplication across API methods.
//!
//! - [`HttpResponseClassifier`]: Classifies HTTP error responses as either infrastructure errors
//!   (retryable) or API errors (non-retryable). This enables smart retry logic that only
//!   retries transient failures.
//!
//! # Architecture
//!
//! This is the lowest layer in the three-layer architecture:
//!
//! ```text
//! ┌─────────────────────────────┐
//! │      Client Layer           │  ← Retry logic, metrics, public API
//! ├─────────────────────────────┤
//! │       API Layer             │  ← Request building, response parsing
//! ├─────────────────────────────┤
//! │    Transport Layer (here)   │  ← Authentication, error classification
//! └─────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use crate::transport::{ApiKeyAuth, HttpResponseClassifier};
//!
//! // Create authenticated request
//! let auth = ApiKeyAuth::new("my-api-key")?;
//! let request = client.request()
//!     .header(ApiKeyAuth::header_name(), auth.header_value());
//!
//! // Classify error response
//! let is_retryable = HttpResponseClassifier::is_infrastructure_error(status, &response_text);
//! ```

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
        response_text.contains("404 page not found")
            || response_text.contains("502 Bad Gateway")
            || response_text.contains("503 Service Unavailable")
            || response_text.contains("504 Gateway Timeout")
            || response_text.contains("<html")
            || response_text.contains("<!DOCTYPE")
            // Weird JSON-encoded string format from stress testing
            // e.g., {"0":"4","1":"0","2":"4",...} encoding "404 page not found"
            || (response_text.starts_with("{\"0\":")
                && response_text.contains("\"4\"")
                && response_text.contains("\"0\""))
            // 404 from infrastructure (not a real Atlantic API 404)
            // Real Atlantic 404s would have proper JSON with "error" field
            || (status == StatusCode::NOT_FOUND
                && !response_text.contains("bucket")
                && !response_text.contains("query")
                && !response_text.contains("job")
                && !response_text.contains("error")
                && !response_text.contains("message"))
            // 502/503/504 are almost always infrastructure issues
            || status == StatusCode::BAD_GATEWAY
            || status == StatusCode::SERVICE_UNAVAILABLE
            || status == StatusCode::GATEWAY_TIMEOUT
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
            // Real API 404 should NOT be classified as infrastructure error
            assert!(!HttpResponseClassifier::is_infrastructure_error(
                StatusCode::NOT_FOUND,
                r#"{"error": "bucket not found"}"#
            ));
        }
    }
}
