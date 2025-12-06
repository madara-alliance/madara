/// Generic resilience primitives for connection health tracking and retry logic.
///
/// This crate provides reusable components for building resilient external service
/// clients. It includes:
///
/// - **Health Tracking**: Monitor connection health with state machine (Healthy → Degraded → Down)
/// - **Retry Strategy**: Phase-based retry with aggressive, backoff, and steady phases
/// - **Adaptive Logging**: Prevent log spam during outages with throttled heartbeats
///
/// # Example: Gateway Client
///
/// ```rust,ignore
/// use mp_resilience::{ConnectionHealth, RetryState, RetryConfig, start_health_monitor};
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// let health = Arc::new(RwLock::new(ConnectionHealth::new("Gateway")));
/// start_health_monitor(Arc::clone(&health));
///
/// let mut retry_state = RetryState::new(RetryConfig::default());
///
/// loop {
///     match make_request().await {
///         Ok(result) => {
///             health.write().await.report_success();
///             return Ok(result);
///         }
///         Err(e) => {
///             health.write().await.report_failure("request");
///             retry_state.increment_retry();
///             let delay = retry_state.next_delay();
///             tokio::time::sleep(delay).await;
///         }
///     }
/// }
/// ```
pub mod health;
pub mod retry;

// Re-export main types for convenience
pub use health::{start_health_monitor, start_health_monitor_with_cancellation, ConnectionHealth, HealthState};
pub use retry::{RetryConfig, RetryPhase, RetryState};
pub use tokio_util::sync::CancellationToken;
