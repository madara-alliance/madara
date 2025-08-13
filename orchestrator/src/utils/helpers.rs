use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// wait_until_ready - Wait until the provided function returns a result or the timeout is reached
/// It will return the result of the function or an error if the timeout is reached
/// # Arguments
/// * `f` - The function to call
/// * `timeout_secs` - The timeout in seconds
/// # Returns
/// * `Result<T, E>` - The result of the function or an error if the timeout is reached
/// # Examples
/// ```
/// use std::time::Duration;
/// use orchestrator::utils::helpers::wait_until_ready;
///
/// async fn wait_for_ready() -> Result<(), Box<dyn std::error::Error>> {
///     let result = wait_until_ready(|| async { Ok(()) }, 10).await;
///     assert!(result.is_ok());
///     Ok(())
/// }
/// ```
pub async fn wait_until_ready<F, T, E>(mut f: F, timeout_secs: u64) -> Result<T, E>
where
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    let start = tokio::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(_) if start.elapsed() < timeout => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// retry_async - Retry an async function up to a specified number of times with optional delay between retries
///
/// # Arguments
/// * `mut f` - The async function to call (should return Result<T, E>)
/// * `max_retries` - Maximum number of attempts (including the first)
/// * `delay` - Optional delay between retries (Duration)
///
/// # Returns
/// * `Result<T, E>` - The result of the function or the last error if all retries fail
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use orchestrator::utils::helpers::retry_async;
///
/// async fn might_fail() -> Result<u32, &'static str> {
///     Err("fail")
/// }
///
/// // let result = retry_async(|| Box::pin(might_fail()), 3, Some(Duration::from_secs(1)));
/// // assert!(result.is_err());
/// ```
pub async fn retry_async<F, Fut, T, E>(mut func: F, max_retries: u64, delay: Option<Duration>) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempts = 0;
    loop {
        match func().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempts += 1;
                if attempts >= max_retries {
                    return Err(e);
                }
                if let Some(d) = delay {
                    tokio::time::sleep(d).await;
                }
            }
        }
    }
}
