use reqwest::StatusCode;

/// Classifies HTTP responses for retry decisions.
///
/// Distinguishes between infrastructure errors (load balancer, gateway)
/// which should be retried, and real API errors which should not.
pub struct HttpResponseClassifier;

impl HttpResponseClassifier {
    /// Determines if an HTTP error response indicates an infrastructure issue.
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
    pub fn is_infrastructure_error(status: StatusCode, response_text: &str) -> bool {
        if response_text.contains("404 page not found")
            || response_text.contains("502 Bad Gateway")
            || response_text.contains("503 Service Unavailable")
            || response_text.contains("504 Gateway Timeout")
            || response_text.contains("<html")
            || response_text.contains("<!DOCTYPE")
        {
            return true;
        }

        if response_text.starts_with("{\"0\":") && response_text.contains("\"4\"") && response_text.contains("\"0\"") {
            return true;
        }

        if status == StatusCode::BAD_GATEWAY
            || status == StatusCode::SERVICE_UNAVAILABLE
            || status == StatusCode::GATEWAY_TIMEOUT
        {
            return true;
        }

        if status == StatusCode::NOT_FOUND {
            if serde_json::from_str::<serde_json::Value>(response_text).is_ok() {
                return false;
            }

            let has_api_keywords = response_text.contains("bucket")
                || response_text.contains("query")
                || response_text.contains("job")
                || response_text.contains("error")
                || response_text.contains("message");

            return !has_api_keywords;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    // Infrastructure errors — should be retried
    #[case::gateway_404("404 page not found", StatusCode::NOT_FOUND, true)]
    #[case::bad_gateway("502 Bad Gateway", StatusCode::BAD_GATEWAY, true)]
    #[case::service_unavailable("503 Service Unavailable", StatusCode::SERVICE_UNAVAILABLE, true)]
    #[case::gateway_timeout("504 Gateway Timeout", StatusCode::GATEWAY_TIMEOUT, true)]
    #[case::nginx_json_404(r#"{"0":"4","1":"0","2":"4"}"#, StatusCode::NOT_FOUND, true)]
    #[case::plain_text_404("Not Found", StatusCode::NOT_FOUND, true)]
    #[case::html_404("<html><body>404 Not Found</body></html>", StatusCode::NOT_FOUND, true)]
    // API errors — should NOT be retried
    #[case::json_bucket_404(r#"{"error": "bucket not found"}"#, StatusCode::NOT_FOUND, false)]
    #[case::json_structured_404(r#"{"status": "not_found", "code": 404}"#, StatusCode::NOT_FOUND, false)]
    fn test_http_response_classification(#[case] body: &str, #[case] status: StatusCode, #[case] expected_infra: bool) {
        assert_eq!(
            HttpResponseClassifier::is_infrastructure_error(status, body),
            expected_infra,
            "status={status}, body={body:?}"
        );
    }
}
