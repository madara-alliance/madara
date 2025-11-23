/// Gateway-specific retry strategy with error handling
///
/// This module wraps the generic mp-resilience retry logic with gateway-specific
/// error type handling for SequencerError.
use mp_gateway::error::{SequencerError, StarknetErrorCode};
use std::time::Duration;

// Re-export the generic retry types
pub use mp_resilience::{RetryConfig, RetryPhase, RetryState};

/// Gateway-specific retry state extensions
pub struct GatewayRetryState {
    inner: RetryState,
}

impl GatewayRetryState {
    pub fn new(config: RetryConfig) -> Self {
        Self { inner: RetryState::new(config) }
    }

    /// Calculate delay for next retry based on current phase and error type
    pub fn next_delay(&self, error: &SequencerError) -> Duration {
        // Handle rate limiting separately
        if self.is_rate_limited(error) {
            return self.extract_retry_after(error).unwrap_or(Duration::from_secs(10));
        }

        self.inner.next_delay()
    }

    /// Check if we should log this retry attempt (throttled logging)
    pub fn should_log(&mut self) -> bool {
        self.inner.should_log()
    }

    /// Increment retry counter and return current count
    pub fn increment_retry(&mut self) -> usize {
        self.inner.increment_retry()
    }

    /// Get current retry count
    #[allow(dead_code)]
    pub fn get_retry_count(&self) -> usize {
        self.inner.get_retry_count()
    }

    /// Determine current retry phase based on elapsed time
    pub fn current_phase(&self) -> RetryPhase {
        self.inner.current_phase()
    }

    /// Get elapsed time since first retry
    #[allow(dead_code)]
    pub fn elapsed(&self) -> Duration {
        self.inner.elapsed()
    }

    /// Check if an error is retryable
    ///
    /// Retryable errors are transient network issues that may succeed on retry:
    /// - HTTP/network errors (connection refused, timeout, etc.)
    /// - Rate limiting errors
    ///
    /// Non-retryable errors are valid API responses that won't change on retry:
    /// - NoBlockHeader, BlockNotFound, UndeclaredClass, etc.
    pub fn is_retryable(error: &SequencerError) -> bool {
        match error {
            // Network-level errors are always retryable
            SequencerError::HttpCallError(_) | SequencerError::HyperError(_) => true,

            // For StarknetError, only rate limits are retryable
            // All other StarknetErrors are valid API responses
            SequencerError::StarknetError(e) => matches!(e.code, StarknetErrorCode::RateLimited),

            // Other error types are retryable by default
            _ => true,
        }
    }

    /// Check if error is a rate limit error
    fn is_rate_limited(&self, error: &SequencerError) -> bool {
        matches!(
            error,
            SequencerError::StarknetError(e) if e.code == StarknetErrorCode::RateLimited
        )
    }

    /// Extract Retry-After duration from error if available
    fn extract_retry_after(&self, _error: &SequencerError) -> Option<Duration> {
        // TODO: Parse Retry-After header from HttpCallError if available
        // For now, return None and use default rate limit delay
        None
    }

    /// Check if error is a connection error (network-level failure)
    #[allow(dead_code)]
    pub fn is_connection_error(error: &SequencerError) -> bool {
        match error {
            SequencerError::HttpCallError(e) => {
                let error_str = e.to_string().to_lowercase();
                error_str.contains("connection refused")
                    || error_str.contains("network unreachable")
                    || error_str.contains("connection reset")
                    || error_str.contains("broken pipe")
            }
            SequencerError::HyperError(_) => true,
            _ => false,
        }
    }

    /// Check if error is a timeout
    #[allow(dead_code)]
    pub fn is_timeout_error(error: &SequencerError) -> bool {
        match error {
            SequencerError::HttpCallError(e) => {
                let error_str = e.to_string().to_lowercase();
                error_str.contains("timeout") || error_str.contains("timed out")
            }
            _ => false,
        }
    }

    /// Format error for user-friendly logging
    pub fn format_error_reason(error: &SequencerError) -> String {
        match error {
            SequencerError::HttpCallError(e) => {
                let error_str = e.to_string();
                if error_str.contains("Connection refused") {
                    "connection refused".to_string()
                } else if error_str.contains("timeout") || error_str.contains("timed out") {
                    "timeout".to_string()
                } else if error_str.contains("network unreachable") {
                    "network unreachable".to_string()
                } else if error_str.contains("connection reset") {
                    "connection reset".to_string()
                } else {
                    "network error".to_string()
                }
            }
            SequencerError::StarknetError(e) if e.code == StarknetErrorCode::RateLimited => "rate limited".to_string(),
            SequencerError::StarknetError(e) => format!("{:?}", e.code).to_lowercase().replace('_', " "),
            SequencerError::HyperError(_) => "http client error".to_string(),
            _ => "unknown error".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RetryConfig::default();
        assert_eq!(config.phase1_duration, Duration::from_secs(5 * 60));
        assert_eq!(config.phase1_interval, Duration::from_secs(2));
        assert_eq!(config.max_backoff, Duration::from_secs(60));
        assert!(config.infinite_retry);
    }

    #[test]
    fn test_phase_determination() {
        let config = RetryConfig { phase1_duration: Duration::from_secs(10), ..Default::default() };
        let state = GatewayRetryState::new(config);

        // Should start in aggressive phase
        assert_eq!(state.current_phase(), RetryPhase::Aggressive);
    }

    #[test]
    fn test_retry_count() {
        let mut state = GatewayRetryState::new(RetryConfig::default());
        assert_eq!(state.get_retry_count(), 0);

        assert_eq!(state.increment_retry(), 1);
        assert_eq!(state.increment_retry(), 2);
        assert_eq!(state.get_retry_count(), 2);
    }

    #[tokio::test]
    async fn test_log_throttling() {
        let config = RetryConfig { log_interval: Duration::from_millis(100), ..Default::default() };
        let mut state = GatewayRetryState::new(config);

        // First log should always be allowed
        assert!(state.should_log());

        // Immediate second log should be throttled
        assert!(!state.should_log());

        // After interval, should log again
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(state.should_log());
    }

    #[test]
    fn test_is_retryable_rate_limit() {
        use mp_gateway::error::StarknetError;

        let rate_limit_error = SequencerError::StarknetError(StarknetError {
            code: StarknetErrorCode::RateLimited,
            message: "Rate limited".to_string(),
        });

        assert!(GatewayRetryState::is_retryable(&rate_limit_error), "Rate limit errors should be retryable");
    }

    #[test]
    fn test_is_retryable_non_retryable_starknet_errors() {
        use mp_gateway::error::StarknetError;

        // Test various non-retryable Starknet errors
        let test_cases = vec![
            (StarknetErrorCode::NoBlockHeader, "NoBlockHeader"),
            (StarknetErrorCode::BlockNotFound, "BlockNotFound"),
            (StarknetErrorCode::UndeclaredClass, "UndeclaredClass"),
            (StarknetErrorCode::NoSignatureForPendingBlock, "NoSignatureForPendingBlock"),
        ];

        for (code, name) in test_cases {
            let error = SequencerError::StarknetError(StarknetError {
                code,
                message: format!("{} error", name),
            });

            assert!(
                !GatewayRetryState::is_retryable(&error),
                "{} should not be retryable",
                name
            );
        }
    }
}
