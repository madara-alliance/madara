/// Generic Connection Health Tracking System
///
/// This module provides centralized health monitoring for external service connections.
/// It tracks overall connection health state and provides clean, aggregated logging
/// instead of per-operation spam.
///
/// Features:
/// - Three health states: Healthy, Degraded, Down
/// - Adaptive heartbeat logging (5s → 10s → 30s based on outage duration)
/// - Recovery confirmation (waits for stable connection before declaring healthy)
/// - Per-operation failure tracking
/// - State transition logging
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

// Health state transition thresholds
const CONSECUTIVE_FAILURES_TO_DOWN: usize = 10;
const FAILURE_RATE_DEGRADED_THRESHOLD: f32 = 0.5;
const CONSECUTIVE_SUCCESSES_FOR_RECOVERY: usize = 3;
const RECOVERY_ATTEMPTS_THRESHOLD: usize = 10;
const FAILURE_RATE_HEALTHY_THRESHOLD: f32 = 0.1;

// Minimum time to stay in Degraded state before transitioning to Healthy
// This prevents rapid state oscillations if connection is flaky
const MIN_TIME_IN_DEGRADED: Duration = Duration::from_secs(2);

// Heartbeat intervals
const HEARTBEAT_INTERVAL_DEGRADED: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL_DOWN_PHASE1: Duration = Duration::from_secs(5); // 0-5 minutes
const HEARTBEAT_INTERVAL_DOWN_PHASE2: Duration = Duration::from_secs(10); // 5-30 minutes
const HEARTBEAT_INTERVAL_DOWN_PHASE3: Duration = Duration::from_secs(60); // 30+ minutes

// Phase durations for adaptive logging
const PHASE1_DURATION: Duration = Duration::from_secs(5 * 60); // 5 minutes
const PHASE2_DURATION: Duration = Duration::from_secs(30 * 60); // 30 minutes

/// Connection health state
#[derive(Debug, Clone, PartialEq)]
pub enum HealthState {
    /// All operations succeeding normally
    Healthy,

    /// Some operations failing, some succeeding (intermittent issues)
    Degraded { failure_rate: f32 },

    /// All or most operations failing (connection down or unreachable)
    Down,
}

/// Generic connection health tracker
#[derive(Debug)]
pub struct ConnectionHealth {
    /// Name of the service being tracked (e.g., "Gateway", "L1 Endpoint")
    service_name: Arc<str>,

    /// Current health state
    state: HealthState,

    /// When the first failure occurred (start of degradation/downtime)
    first_failure_time: Option<Instant>,

    /// When the state last changed
    last_state_change: Instant,

    /// When we last logged a heartbeat message
    last_heartbeat_log: Option<Instant>,

    /// Total requests since last state transition
    total_requests: usize,

    /// Failed requests since last state transition
    failed_requests: usize,

    /// Consecutive successful requests
    consecutive_successes: usize,

    /// Consecutive failed requests
    consecutive_failures: usize,

    /// Per-operation failure tracking
    failed_operations: HashMap<String, usize>,

    /// Recovery attempts (successes since entering recovery)
    recovery_attempts: usize,
}

impl ConnectionHealth {
    pub fn new(service_name: impl Into<Arc<str>>) -> Self {
        Self {
            service_name: service_name.into(),
            state: HealthState::Healthy,
            first_failure_time: None,
            last_state_change: Instant::now(),
            last_heartbeat_log: None,
            total_requests: 0,
            failed_requests: 0,
            consecutive_successes: 0,
            consecutive_failures: 0,
            failed_operations: HashMap::new(),
            recovery_attempts: 0,
        }
    }

    /// Report a failed operation
    pub fn report_failure(&mut self, operation: &str) {
        self.total_requests += 1;
        self.failed_requests += 1;
        self.consecutive_failures += 1;
        self.consecutive_successes = 0;

        *self.failed_operations.entry(operation.to_string()).or_insert(0) += 1;

        // Prevent unbounded memory growth: limit to top 50 failing operations
        if self.failed_operations.len() > 50 {
            // Keep only the 20 most frequently failing operations
            let mut ops: Vec<_> = self.failed_operations.iter().map(|(k, v)| (k.clone(), *v)).collect();
            ops.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by failure count descending
            self.failed_operations = ops.into_iter().take(20).collect();
        }

        self.transition_on_failure();
    }

    /// Handle state transitions on failure
    fn transition_on_failure(&mut self) {
        match &self.state {
            HealthState::Healthy => self.transition_healthy_to_degraded(),
            HealthState::Degraded { .. } if self.should_transition_to_down() => self.transition_degraded_to_down(),
            _ => {}
        }
    }

    fn transition_healthy_to_degraded(&mut self) {
        self.state = HealthState::Degraded { failure_rate: self.failure_rate() };
        self.first_failure_time = Some(Instant::now());
        self.last_state_change = Instant::now();
        tracing::warn!("🟡 {} experiencing intermittent errors", self.service_name);
    }

    fn transition_degraded_to_down(&mut self) {
        self.state = HealthState::Down;
        self.last_state_change = Instant::now();
        self.recovery_attempts = 0;
        tracing::warn!("🔴 {} connection lost - retrying...", self.service_name);
    }

    fn should_transition_to_down(&self) -> bool {
        self.consecutive_failures >= CONSECUTIVE_FAILURES_TO_DOWN
            || self.failure_rate() > FAILURE_RATE_DEGRADED_THRESHOLD
    }

    /// Report a successful operation
    pub fn report_success(&mut self) {
        self.total_requests += 1;
        self.consecutive_successes += 1;
        self.consecutive_failures = 0;

        self.transition_on_success();
    }

    /// Handle state transitions on success
    fn transition_on_success(&mut self) {
        match &self.state {
            HealthState::Healthy => {}
            HealthState::Down => self.transition_down_to_degraded(),
            HealthState::Degraded { .. } => self.try_transition_to_healthy(),
        }
    }

    fn transition_down_to_degraded(&mut self) {
        let downtime = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(0));
        let failed_ops = self.failed_requests;

        // Reset counters to reflect current state, not historical outage
        self.total_requests = 1; // The success that triggered this transition
        self.failed_requests = 0;
        self.consecutive_failures = 0;
        self.consecutive_successes = 1;
        self.failed_operations.clear();

        self.state = HealthState::Degraded { failure_rate: 0.0 };
        self.recovery_attempts = 1;
        self.last_state_change = Instant::now();

        tracing::info!(
            "🟡 {} partially restored - monitoring stability... (was down for {}, {} operations failed)",
            self.service_name,
            format_duration(downtime),
            failed_ops
        );

        // Immediately check if we can transition to healthy
        // If the operation that brought us back is successful (which it is),
        // and we have no ongoing failures, transition immediately
        self.try_transition_to_healthy();
    }

    fn try_transition_to_healthy(&mut self) {
        self.recovery_attempts += 1;

        if self.should_transition_to_healthy() {
            let downtime = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(0));
            let failed_ops = self.failed_requests;

            self.state = HealthState::Healthy;
            self.last_state_change = Instant::now();

            tracing::info!(
                "🟢 {} UP - Restored after {} ({} operations failed during outage)",
                self.service_name,
                format_duration(downtime),
                failed_ops
            );

            self.reset_metrics();
        }
    }

    fn should_transition_to_healthy(&self) -> bool {
        // Immediate transition if we have enough consecutive successes
        if self.consecutive_successes >= CONSECUTIVE_SUCCESSES_FOR_RECOVERY {
            return true;
        }

        // Also transition immediately if no operations are failing (clean recovery)
        // This allows fast transition from Down -> Degraded -> Healthy when L1 comes back up
        if self.failed_operations.is_empty() && self.failure_rate() < f32::EPSILON && self.recovery_attempts > 0 {
            return true;
        }

        // For ongoing partial failures, ensure we've been in Degraded state for minimum time
        // This prevents rapid Down -> Degraded -> Healthy -> Down cycles on flaky connections
        if self.last_state_change.elapsed() < MIN_TIME_IN_DEGRADED {
            return false;
        }

        // Standard recovery: enough attempts with low failure rate
        self.recovery_attempts >= RECOVERY_ATTEMPTS_THRESHOLD && self.failure_rate() < FAILURE_RATE_HEALTHY_THRESHOLD
    }

    /// Calculate current failure rate
    fn failure_rate(&self) -> f32 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.failed_requests as f32 / self.total_requests as f32
        }
    }

    /// Check if we should log a heartbeat based on current state
    pub fn should_log_heartbeat(&self) -> bool {
        let interval = match &self.state {
            HealthState::Healthy => return false, // Don't log when healthy
            HealthState::Degraded { .. } => HEARTBEAT_INTERVAL_DEGRADED,
            HealthState::Down => self.get_down_heartbeat_interval(),
        };

        match self.last_heartbeat_log {
            None => true,
            Some(last) => last.elapsed() >= interval,
        }
    }

    /// Get adaptive heartbeat interval for Down state based on outage duration
    fn get_down_heartbeat_interval(&self) -> Duration {
        let elapsed = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::ZERO);

        if elapsed < PHASE1_DURATION {
            HEARTBEAT_INTERVAL_DOWN_PHASE1
        } else if elapsed < PHASE2_DURATION {
            HEARTBEAT_INTERVAL_DOWN_PHASE2
        } else {
            HEARTBEAT_INTERVAL_DOWN_PHASE3
        }
    }

    /// Log current status (called by heartbeat task)
    pub fn log_status(&mut self) {
        match &self.state {
            HealthState::Healthy => {
                // No logging - silence is golden
            }

            HealthState::Degraded { failure_rate } => {
                let duration = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(0));
                let affected_ops: Vec<_> = self.failed_operations.keys().map(|s| s.as_str()).collect();

                // Don't log if no operations are affected (empty list means we're recovering)
                if !affected_ops.is_empty() {
                    tracing::warn!(
                        "🟡 {} unstable ({}) - {:.0}% failure rate, operations affected: {}",
                        self.service_name,
                        format_duration(duration),
                        failure_rate * 100.0,
                        affected_ops.join(", ")
                    );
                }
            }

            HealthState::Down => {
                let duration = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(0));
                let phase = get_retry_phase(duration);

                tracing::warn!(
                    "🔴 {} down ({}) - Phase: {} → {} failed operations",
                    self.service_name,
                    format_duration(duration),
                    phase,
                    self.failed_requests
                );
            }
        }

        self.last_heartbeat_log = Some(Instant::now());
    }

    /// Reset metrics (called when transitioning to healthy)
    fn reset_metrics(&mut self) {
        self.first_failure_time = None;
        self.total_requests = 0;
        self.failed_requests = 0;
        self.consecutive_successes = 0;
        self.consecutive_failures = 0;
        self.failed_operations.clear();
        self.recovery_attempts = 0;
    }

    /// Get current health state
    pub fn state(&self) -> &HealthState {
        &self.state
    }
}

/// Start a background health monitor task
///
/// This spawns a tokio task that periodically logs connection health status.
/// The returned JoinHandle can be used to stop the monitor during graceful shutdown.
///
/// # Arguments
/// * `health` - Arc to the ConnectionHealth instance to monitor
///
/// # Returns
/// A JoinHandle that can be awaited or aborted to stop the monitor
pub fn start_health_monitor(health: Arc<RwLock<ConnectionHealth>>) -> tokio::task::JoinHandle<()> {
    start_health_monitor_with_cancellation(health, CancellationToken::new())
}

/// Start a background health monitor task with cancellation support
///
/// This spawns a tokio task that periodically logs connection health status.
/// The returned JoinHandle can be used to stop the monitor during graceful shutdown.
///
/// # Arguments
/// * `health` - Arc to the ConnectionHealth instance to monitor
/// * `cancellation_token` - Token to signal graceful shutdown
///
/// # Returns
/// A JoinHandle that can be awaited or aborted to stop the monitor
pub fn start_health_monitor_with_cancellation(
    health: Arc<RwLock<ConnectionHealth>>,
    cancellation_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let service_name = {
            let guard = health.read().await;
            guard.service_name.clone()
        };

        tracing::debug!("{} health monitor started", service_name);

        loop {
            // Use tokio::select! to make the sleep cancellable and support graceful shutdown
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    tracing::debug!("{} health monitor shutting down", service_name);
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    let mut health_guard = health.write().await;

                    if health_guard.should_log_heartbeat() {
                        health_guard.log_status();
                    }
                }
            }
        }
    })
}

/// Format duration in human-readable form
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        let rem_secs = secs % 60;
        if rem_secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m{}s", mins, rem_secs)
        }
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        if mins == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h{}m", hours, mins)
        }
    }
}

/// Get retry phase name based on duration
fn get_retry_phase(duration: Duration) -> &'static str {
    let secs = duration.as_secs();
    if secs < 5 * 60 {
        "Aggressive"
    } else if secs < 30 * 60 {
        "Backoff"
    } else {
        "Steady"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(5)), "5s");
        assert_eq!(format_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(Duration::from_secs(65)), "1m5s");
        assert_eq!(format_duration(Duration::from_secs(135)), "2m15s");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(3660)), "1h1m");
        assert_eq!(format_duration(Duration::from_secs(5400)), "1h30m");
    }

    #[test]
    fn test_get_retry_phase() {
        assert_eq!(get_retry_phase(Duration::from_secs(30)), "Aggressive");
        assert_eq!(get_retry_phase(Duration::from_secs(299)), "Aggressive");
        assert_eq!(get_retry_phase(Duration::from_secs(300)), "Backoff");
        assert_eq!(get_retry_phase(Duration::from_secs(1000)), "Backoff");
        assert_eq!(get_retry_phase(Duration::from_secs(1799)), "Backoff");
        assert_eq!(get_retry_phase(Duration::from_secs(1800)), "Steady");
        assert_eq!(get_retry_phase(Duration::from_secs(5000)), "Steady");
    }

    #[test]
    fn test_state_transitions() {
        let mut health = ConnectionHealth::new("TestService");

        // Start healthy
        assert!(matches!(health.state, HealthState::Healthy));

        // First failure -> Degraded
        health.report_failure("test_op");
        assert!(matches!(health.state, HealthState::Degraded { .. }));

        // More failures -> Down
        for _ in 0..10 {
            health.report_failure("test_op");
        }
        assert!(matches!(health.state, HealthState::Down));

        // First success -> Immediately transitions to Healthy (clean recovery with no ongoing failures)
        health.report_success();
        assert!(matches!(health.state, HealthState::Healthy));
    }

    #[test]
    fn test_failure_rate() {
        let mut health = ConnectionHealth::new("TestService");

        health.report_success();
        health.report_failure("test");
        assert!((health.failure_rate() - 0.5).abs() < 0.01);

        health.report_success();
        assert!((health.failure_rate() - 0.33).abs() < 0.02);
    }
}
