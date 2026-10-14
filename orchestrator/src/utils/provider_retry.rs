use std::future::Future;
use std::time::Duration;

use rand::Rng;
use starknet::providers::ProviderError;
use thiserror::Error;
use tokio::time::timeout;
use tracing::warn;

const PROVIDER_READ_RETRY_CONFIG: ProviderReadRetryConfig = ProviderReadRetryConfig {
    max_attempts: 3,
    timeout: Duration::from_secs(30),
    initial_backoff: Duration::from_millis(200),
};

#[derive(Clone, Copy)]
struct ProviderReadRetryConfig {
    max_attempts: usize,
    timeout: Duration,
    initial_backoff: Duration,
}

#[derive(Debug, Error)]
pub enum ProviderReadError {
    #[error("Provider request timed out after {timeout:?}")]
    Timeout { timeout: Duration },
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

pub async fn retry_provider_read<F, Fut, T>(operation: &'static str, request: F) -> Result<T, ProviderReadError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    retry_provider_read_with_config(operation, PROVIDER_READ_RETRY_CONFIG, request).await
}

async fn retry_provider_read_with_config<F, Fut, T>(
    operation: &'static str,
    config: ProviderReadRetryConfig,
    mut request: F,
) -> Result<T, ProviderReadError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    let mut backoff = config.initial_backoff;

    for attempt in 1..=config.max_attempts {
        match timeout(config.timeout, request()).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(error)) => {
                if attempt == config.max_attempts || !is_retryable_provider_error(&error) {
                    return Err(error.into());
                }
                warn!(operation, attempt, max_attempts = config.max_attempts, error = %error, "Retrying Madara provider read");
            }
            Err(_) => {
                if attempt == config.max_attempts {
                    return Err(ProviderReadError::Timeout { timeout: config.timeout });
                }
                warn!(operation, attempt, max_attempts = config.max_attempts, timeout = ?config.timeout, "Retrying timed out Madara provider read");
            }
        }

        tokio::time::sleep(backoff + jitter(backoff)).await;
        backoff = backoff.saturating_mul(2);
    }

    unreachable!("retry loop returns on final attempt")
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use starknet::providers::ProviderError;

    use super::{retry_provider_read_with_config, ProviderReadError, ProviderReadRetryConfig};

    #[tokio::test]
    async fn retries_retryable_errors_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let result = retry_provider_read_with_config("test", test_config(), || {
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
        let result = retry_provider_read_with_config("test", test_config(), || {
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
        let result = retry_provider_read_with_config("test", test_config(), || {
            let attempts = attempts.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok::<(), ProviderError>(())
            }
        })
        .await;

        assert!(matches!(result, Err(ProviderReadError::Timeout { .. })));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    fn test_config() -> ProviderReadRetryConfig {
        ProviderReadRetryConfig { max_attempts: 3, timeout: Duration::from_millis(1), initial_backoff: Duration::ZERO }
    }
}
