/// Unit tests for gateway-specific retry error handling
use crate::retry::{GatewayRetryState, RetryConfig};
use mp_gateway::error::SequencerError;
use std::time::Duration;

#[tokio::test]
async fn test_connection_refused_error() {
    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "Connection refused",
    )));

    assert!(GatewayRetryState::is_connection_error(&error));

    let reason = GatewayRetryState::format_error_reason(&error);
    assert_eq!(reason, "connection refused");
}

#[tokio::test]
async fn test_timeout_error() {
    let error = SequencerError::HttpCallError(Box::new(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "Operation timed out",
    )));

    assert!(GatewayRetryState::is_timeout_error(&error));

    let reason = GatewayRetryState::format_error_reason(&error);
    assert_eq!(reason, "timeout");
}

#[tokio::test]
async fn test_rate_limit_handling() {
    use mp_gateway::error::StarknetError;

    let error = SequencerError::StarknetError(StarknetError::rate_limited());

    let state = GatewayRetryState::new(RetryConfig::default());
    let delay = state.next_delay(&error);

    // Should respect rate limiting
    assert!(delay >= Duration::from_secs(2), "Rate limited errors should have appropriate delay");
}

#[tokio::test]
async fn test_error_message_formatting() {
    let test_cases = vec![
        (
            SequencerError::HttpCallError(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Connection refused",
            ))),
            "connection refused",
        ),
        (
            SequencerError::HttpCallError(Box::new(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))),
            "timeout",
        ),
        (SequencerError::StarknetError(mp_gateway::error::StarknetError::rate_limited()), "rate limited"),
    ];

    for (error, expected) in test_cases {
        let formatted = GatewayRetryState::format_error_reason(&error);
        assert_eq!(formatted, expected, "Error formatting mismatch for: {:?}", error);
    }
}

#[tokio::test]
async fn test_mixed_error_types() {
    let errors = vec![
        SequencerError::HttpCallError(Box::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Connection refused",
        ))),
        SequencerError::HttpCallError(Box::new(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))),
        SequencerError::StarknetError(mp_gateway::error::StarknetError::rate_limited()),
    ];

    let state = GatewayRetryState::new(RetryConfig::default());

    for error in errors {
        let delay = state.next_delay(&error);
        let is_conn_error = GatewayRetryState::is_connection_error(&error);
        let is_timeout = GatewayRetryState::is_timeout_error(&error);
        let formatted = GatewayRetryState::format_error_reason(&error);

        // Verify basic properties
        assert!(delay > Duration::from_secs(0), "Should have non-zero delay");

        // At least one classification should match
        assert!(is_conn_error || is_timeout || !formatted.is_empty(), "Error should be classified: {:?}", error);
    }
}
