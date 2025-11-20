/// Hybrid retry strategy with phase-based backoff for gateway requests.
///
/// This module implements a sophisticated retry mechanism that adapts to different
/// failure scenarios:
///
/// - **Phase 1 (Aggressive)**: 0-5 minutes - Quick recovery for temporary blips
/// - **Phase 2 (Backoff)**: 5-30 minutes - Exponential backoff for prolonged outages
/// - **Phase 3 (Steady)**: 30+ minutes - Fixed polling for extended maintenance
///
/// The strategy also considers error types (connection refused, timeout, rate limits)
/// to optimize retry behavior.

use mp_gateway::error::{SequencerError, StarknetErrorCode};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[cfg(not(test))]
use std::time::Instant;

#[cfg(test)]
use tokio::time::Instant as TokioInstant;

/// Configuration for the hybrid retry strategy
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Duration of Phase 1 (aggressive retry phase)
    pub phase1_duration: Duration,
    /// Retry interval during Phase 1
    pub phase1_interval: Duration,
    /// Minimum delay for Phase 2 exponential backoff
    pub phase2_min_delay: Duration,
    /// Maximum backoff delay (cap for exponential growth)
    pub max_backoff: Duration,
    /// Interval for logging warnings during retries
    pub log_interval: Duration,
    /// Whether to enable infinite retries (true for full nodes)
    pub infinite_retry: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            phase1_duration: Duration::from_secs(5 * 60), // 5 minutes
            phase1_interval: Duration::from_secs(2),      // 2 seconds
            phase2_min_delay: Duration::from_secs(5),     // 5 seconds
            max_backoff: Duration::from_secs(60),         // 60 seconds (1 minute)
            log_interval: Duration::from_secs(10),        // Log every 10 seconds
            infinite_retry: true,                         // Full nodes should retry indefinitely
        }
    }
}

/// Represents the current phase of the retry strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPhase {
    /// Phase 1: Aggressive retry (0-5 minutes)
    Aggressive,
    /// Phase 2: Exponential backoff (5-30 minutes)
    Backoff { attempt_in_phase: u32 },
    /// Phase 3: Steady state polling (30+ minutes)
    Steady,
}

/// State tracker for retry attempts
pub struct RetryState {
    config: RetryConfig,
    #[cfg(not(test))]
    start_time: Instant,
    #[cfg(test)]
    start_time: TokioInstant,
    #[cfg(not(test))]
    last_log_time: Arc<RwLock<Option<Instant>>>,
    #[cfg(test)]
    last_log_time: Arc<RwLock<Option<TokioInstant>>>,
    retry_count: Arc<RwLock<usize>>,
}

impl std::fmt::Debug for RetryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryState")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl RetryState {
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            #[cfg(not(test))]
            start_time: Instant::now(),
            #[cfg(test)]
            start_time: TokioInstant::now(),
            last_log_time: Arc::new(RwLock::new(None)),
            retry_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Determine current retry phase based on elapsed time
    pub fn current_phase(&self) -> RetryPhase {
        let elapsed = self.start_time.elapsed();

        if elapsed < self.config.phase1_duration {
            RetryPhase::Aggressive
        } else {
            let time_in_phase2 = elapsed - self.config.phase1_duration;
            let phase2_attempts = (time_in_phase2.as_secs() / self.config.phase2_min_delay.as_secs()) as u32;
            RetryPhase::Backoff { attempt_in_phase: phase2_attempts }
        }
    }

    /// Calculate delay for next retry based on current phase and error type
    pub fn next_delay(&self, error: &SequencerError) -> Duration {
        match self.current_phase() {
            RetryPhase::Aggressive => {
                // Phase 1: Fixed interval, but respect rate limiting
                if self.is_rate_limited(error) {
                    self.extract_retry_after(error).unwrap_or(Duration::from_secs(10))
                } else {
                    self.config.phase1_interval
                }
            }
            RetryPhase::Backoff { attempt_in_phase } => {
                // Phase 2: Exponential backoff with cap
                if self.is_rate_limited(error) {
                    self.extract_retry_after(error).unwrap_or(Duration::from_secs(10))
                } else {
                    let exponential_delay = self.config.phase2_min_delay
                        .saturating_mul(2_u32.saturating_pow(attempt_in_phase));
                    exponential_delay.min(self.config.max_backoff)
                }
            }
            RetryPhase::Steady => {
                // Phase 3: Fixed max backoff
                self.config.max_backoff
            }
        }
    }

    /// Check if we should log this retry attempt (throttled logging)
    pub async fn should_log(&self) -> bool {
        let mut last_log = self.last_log_time.write().await;
        match *last_log {
            None => {
                #[cfg(not(test))]
                {
                    *last_log = Some(Instant::now());
                }
                #[cfg(test)]
                {
                    *last_log = Some(TokioInstant::now());
                }
                true
            }
            Some(last) => {
                if last.elapsed() >= self.config.log_interval {
                    #[cfg(not(test))]
                    {
                        *last_log = Some(Instant::now());
                    }
                    #[cfg(test)]
                    {
                        *last_log = Some(TokioInstant::now());
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Increment retry counter and return current count
    pub async fn increment_retry(&self) -> usize {
        let mut count = self.retry_count.write().await;
        *count += 1;
        *count
    }

    /// Get current retry count
    pub async fn get_retry_count(&self) -> usize {
        *self.retry_count.read().await
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
            SequencerError::StarknetError(e) if e.code == StarknetErrorCode::RateLimited => {
                "rate limited".to_string()
            }
            SequencerError::StarknetError(e) => {
                format!("{:?}", e.code).to_lowercase().replace('_', " ")
            }
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
        let config = RetryConfig {
            phase1_duration: Duration::from_secs(10),
            ..Default::default()
        };
        let state = RetryState::new(config);

        // Should start in aggressive phase
        assert_eq!(state.current_phase(), RetryPhase::Aggressive);
    }

    #[tokio::test]
    async fn test_retry_count() {
        let state = RetryState::new(RetryConfig::default());
        assert_eq!(state.get_retry_count().await, 0);

        assert_eq!(state.increment_retry().await, 1);
        assert_eq!(state.increment_retry().await, 2);
        assert_eq!(state.get_retry_count().await, 2);
    }

    #[tokio::test]
    async fn test_log_throttling() {
        let config = RetryConfig {
            log_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let state = RetryState::new(config);

        // First log should always be allowed
        assert!(state.should_log().await);

        // Immediate second log should be throttled
        assert!(!state.should_log().await);

        // After interval, should log again
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(state.should_log().await);
    }
}
