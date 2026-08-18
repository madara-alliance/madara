use std::fmt::{Debug, Display};
use std::future::Future;
use std::num::NonZeroUsize;
use std::time::Duration;

use rand::Rng;
use reqwest::{Response, StatusCode};
use starknet::providers::ProviderError;
use thiserror::Error;
use tokio::time::timeout;
use tracing::warn;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpstreamReadRetryConfig {
    max_attempts: NonZeroUsize,
    timeout: Duration,
    initial_backoff: Duration,
}

impl UpstreamReadRetryConfig {
    pub fn new(max_attempts: NonZeroUsize, timeout: Duration, initial_backoff: Duration) -> Self {
        Self { max_attempts, timeout, initial_backoff }
    }

    pub fn max_attempts(&self) -> usize {
        self.max_attempts.get()
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn initial_backoff(&self) -> Duration {
        self.initial_backoff
    }
}

#[derive(Debug)]
enum ReadRetryError<E> {
    Timeout,
    Request(E),
}

#[derive(Debug, Error)]
pub enum ProviderReadError {
    #[error("Provider request timed out after {timeout:?}")]
    Timeout { timeout: Duration },
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

#[derive(Debug, Error)]
pub enum HttpReadError {
    #[error("HTTP request timed out after {timeout:?}")]
    Timeout { timeout: Duration },
    #[error(transparent)]
    Request(#[from] reqwest::Error),
}

pub async fn retry_provider_read<F, Fut, T>(
    operation: &'static str,
    config: &UpstreamReadRetryConfig,
    request: F,
) -> Result<T, ProviderReadError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    retry_provider_read_with_config(operation, config, request).await
}

async fn retry_provider_read_with_config<F, Fut, T>(
    operation: &'static str,
    config: &UpstreamReadRetryConfig,
    request: F,
) -> Result<T, ProviderReadError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    match retry_read_with_config(operation, config, request, |_| false, is_retryable_provider_error).await {
        Ok(value) => Ok(value),
        Err(ReadRetryError::Timeout) => Err(ProviderReadError::Timeout { timeout: config.timeout }),
        Err(ReadRetryError::Request(error)) => Err(ProviderReadError::Provider(error)),
    }
}

pub async fn retry_http_read<F, Fut>(
    operation: &'static str,
    config: &UpstreamReadRetryConfig,
    request: F,
) -> Result<Response, HttpReadError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = reqwest::Result<Response>>,
{
    match retry_read_with_config(
        operation,
        config,
        request,
        |response| is_retryable_http_status(response.status()),
        is_retryable_http_error,
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(ReadRetryError::Timeout) => Err(HttpReadError::Timeout { timeout: config.timeout }),
        Err(ReadRetryError::Request(error)) => Err(HttpReadError::Request(error)),
    }
}

async fn retry_read_with_config<F, Fut, T, E, RetryResponse, RetryError>(
    operation: &'static str,
    config: &UpstreamReadRetryConfig,
    mut request: F,
    mut should_retry_response: RetryResponse,
    mut should_retry_error: RetryError,
) -> Result<T, ReadRetryError<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Debug + Display,
    RetryResponse: FnMut(&T) -> bool,
    RetryError: FnMut(&E) -> bool,
{
    let mut backoff = config.initial_backoff;

    for attempt in 1..=config.max_attempts.get() {
        match timeout(config.timeout, request()).await {
            Ok(Ok(value)) => {
                if attempt == config.max_attempts.get() || !should_retry_response(&value) {
                    return Ok(value);
                }
                warn!(operation, attempt, max_attempts = config.max_attempts.get(), "Retrying transient read response");
            }
            Ok(Err(error)) => {
                if attempt == config.max_attempts.get() || !should_retry_error(&error) {
                    return Err(ReadRetryError::Request(error));
                }
                warn!(operation, attempt, max_attempts = config.max_attempts.get(), error = ?error, "Retrying transient read error");
            }
            Err(_) => {
                if attempt == config.max_attempts.get() {
                    return Err(ReadRetryError::Timeout);
                }
                warn!(operation, attempt, max_attempts = config.max_attempts.get(), timeout = ?config.timeout, "Retrying timed out read");
            }
        }

        tokio::time::sleep(backoff + jitter(backoff)).await;
        backoff = backoff.saturating_mul(2);
    }

    unreachable!("retry loop returns on final attempt")
}

fn is_retryable_http_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn is_retryable_http_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
}

fn jitter(backoff: Duration) -> Duration {
    let max_jitter_ms = backoff.as_millis().min(100) as u64;
    Duration::from_millis(rand::thread_rng().gen_range(0..=max_jitter_ms))
}

fn is_retryable_provider_error(error: &ProviderError) -> bool {
    match error {
        ProviderError::RateLimited => true,
        ProviderError::Other(error) => {
            let message = error.to_string().to_lowercase();
            [
                "error sending request",
                "transport error",
                "timeout",
                "timed out",
                "connection",
                "connection reset",
                "connection refused",
                "dns",
                "temporarily unavailable",
                "502",
                "503",
                "504",
            ]
            .iter()
            .any(|marker| message.contains(marker))
        }
        ProviderError::StarknetError(_) | ProviderError::ArrayLengthMismatch => false,
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use starknet::providers::ProviderError;

    use reqwest::StatusCode;

    use super::{
        is_retryable_http_status, retry_provider_read_with_config, retry_read_with_config, ProviderReadError,
        UpstreamReadRetryConfig,
    };

    #[tokio::test]
    async fn retries_retryable_errors_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = test_config();
        let result = retry_provider_read_with_config("test", &config, || {
            let attempts = attempts.clone();
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(ProviderError::RateLimited)
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn does_not_retry_non_retryable_errors() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = test_config();
        let result = retry_provider_read_with_config("test", &config, || {
            let attempts = attempts.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(ProviderError::ArrayLengthMismatch)
            }
        })
        .await;

        assert!(matches!(result, Err(ProviderReadError::Provider(ProviderError::ArrayLengthMismatch))));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_timeouts_until_exhausted() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = test_config();
        let result = retry_provider_read_with_config("test", &config, || {
            let attempts = attempts.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<Result<(), ProviderError>>().await
            }
        })
        .await;

        assert!(matches!(result, Err(ProviderReadError::Timeout { .. })));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retries_retryable_responses_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let config = test_config();
        let result = retry_read_with_config(
            "test",
            &config,
            || {
                let attempts = attempts.clone();
                async move {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Ok::<_, &'static str>(StatusCode::SERVICE_UNAVAILABLE)
                    } else {
                        Ok(StatusCode::OK)
                    }
                }
            },
            |status| is_retryable_http_status(*status),
            |_| false,
        )
        .await;

        assert_eq!(result.unwrap(), StatusCode::OK);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retries_rate_limits_and_server_errors_only() {
        assert!(is_retryable_http_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_http_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_http_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_http_status(StatusCode::NOT_FOUND));
    }

    fn test_config() -> UpstreamReadRetryConfig {
        UpstreamReadRetryConfig::new(NonZeroUsize::new(3).unwrap(), Duration::from_millis(1), Duration::ZERO)
    }
}
