use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// wait_until_ready - Wait until the provided function returns a result or the timeout is reached
/// This function will repeatedly call the provided function until it returns a result or the timeout is reached
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
