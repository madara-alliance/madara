use std::future::Future;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

pub const DEFAULT_RETRY_BASE_DELAY: Duration = Duration::from_secs(2);
pub const DEFAULT_RETRY_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub base_delay: Duration,
    pub timeout: Duration,
}

impl RetryConfig {
    pub const fn new(base_delay: Duration, timeout: Duration) -> Self {
        Self { base_delay, timeout }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self::new(DEFAULT_RETRY_BASE_DELAY, DEFAULT_RETRY_TIMEOUT)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryOutcome {
    pub attempts: u32,
    pub elapsed: Duration,
}

impl RetryOutcome {
    pub fn retry_count(&self) -> u32 {
        self.attempts.saturating_sub(1)
    }
}

#[derive(Debug)]
pub struct RetrySuccess<T> {
    pub value: T,
    pub outcome: RetryOutcome,
}

#[derive(Debug)]
pub struct RetryFailure<E> {
    pub error: E,
    pub outcome: RetryOutcome,
}

pub trait RetryableRequestError: std::error::Error + Send + Sync + 'static {
    fn is_retryable(&self) -> bool;
    fn error_type(&self) -> &'static str;
}

pub async fn retry_with_exponential_backoff<F, Fut, T, E>(
    operation_name: &str,
    context: &str,
    config: RetryConfig,
    mut f: F,
) -> Result<RetrySuccess<T>, RetryFailure<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: RetryableRequestError,
{
    let start_time = Instant::now();
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;

        match f().await {
            Ok(value) => {
                let outcome = RetryOutcome { attempts: attempt, elapsed: start_time.elapsed() };

                if attempt > 1 {
                    info!(
                        operation = operation_name,
                        attempts = attempt,
                        retry_count = outcome.retry_count(),
                        duration_ms = outcome.elapsed.as_millis() as u64,
                        "API call succeeded after retry"
                    );
                } else {
                    debug!(
                        operation = operation_name,
                        duration_ms = outcome.elapsed.as_millis() as u64,
                        "API call completed"
                    );
                }

                return Ok(RetrySuccess { value, outcome });
            }
            Err(error) => {
                let outcome = RetryOutcome { attempts: attempt, elapsed: start_time.elapsed() };

                if !error.is_retryable() {
                    warn!(
                        operation = operation_name,
                        error_type = error.error_type(),
                        retry_count = outcome.retry_count(),
                        error = %error,
                        "API call failed (non-retryable)"
                    );
                    return Err(RetryFailure { error, outcome });
                }

                if outcome.elapsed >= config.timeout {
                    warn!(
                        operation = operation_name,
                        duration_seconds = outcome.elapsed.as_secs_f64(),
                        retry_count = outcome.retry_count(),
                        error_type = error.error_type(),
                        context = context,
                        error = %error,
                        "API request failed after retry timeout"
                    );
                    return Err(RetryFailure { error, outcome });
                }

                let delay = config.base_delay.saturating_mul(1u32 << (attempt - 1).min(31));
                warn!(
                    operation = operation_name,
                    attempt = attempt,
                    next_delay_secs = delay.as_secs(),
                    elapsed_secs = outcome.elapsed.as_secs(),
                    timeout_secs = config.timeout.as_secs(),
                    context = context,
                    error = %error,
                    "API call failed, retrying with exponential backoff"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}
