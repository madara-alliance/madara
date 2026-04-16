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

    #[test]
    fn classifies_gateway_404_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::NOT_FOUND, "404 page not found"));
    }

    #[test]
    fn classifies_502_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::BAD_GATEWAY, "502 Bad Gateway"));
    }

    #[test]
    fn classifies_503_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "503 Service Unavailable"
        ));
    }

    #[test]
    fn classifies_504_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::GATEWAY_TIMEOUT, "504 Gateway Timeout"));
    }

    #[test]
    fn classifies_weird_json_encoded_404_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::NOT_FOUND, r#"{"0":"4","1":"0","2":"4"}"#));
    }

    #[test]
    fn keeps_real_json_404_as_api_error() {
        assert!(!HttpResponseClassifier::is_infrastructure_error(
            StatusCode::NOT_FOUND,
            r#"{"error": "bucket not found"}"#
        ));
    }

    #[test]
    fn keeps_structured_json_404_as_api_error() {
        assert!(!HttpResponseClassifier::is_infrastructure_error(
            StatusCode::NOT_FOUND,
            r#"{"status": "not_found", "code": 404}"#
        ));
    }

    #[test]
    fn classifies_plain_text_404_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(StatusCode::NOT_FOUND, "Not Found"));
    }

    #[test]
    fn classifies_html_404_as_infrastructure() {
        assert!(HttpResponseClassifier::is_infrastructure_error(
            StatusCode::NOT_FOUND,
            "<html><body>404 Not Found</body></html>"
        ));
    }
}
