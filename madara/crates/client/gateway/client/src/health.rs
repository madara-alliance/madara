/// Gateway Health Tracking System
///
/// This module provides centralized health monitoring for gateway connections.
/// It tracks overall gateway health state and provides clean, aggregated logging
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
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// Health state transition thresholds
const CONSECUTIVE_FAILURES_TO_DOWN: usize = 10;
const FAILURE_RATE_DEGRADED_THRESHOLD: f32 = 0.5;
const CONSECUTIVE_SUCCESSES_FOR_RECOVERY: usize = 3;
const RECOVERY_ATTEMPTS_THRESHOLD: usize = 10;
const FAILURE_RATE_HEALTHY_THRESHOLD: f32 = 0.1;

// Heartbeat intervals
const HEARTBEAT_INTERVAL_DEGRADED: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL_DOWN_PHASE1: Duration = Duration::from_secs(5); // 0-5 minutes
const HEARTBEAT_INTERVAL_DOWN_PHASE2: Duration = Duration::from_secs(10); // 5-30 minutes
const HEARTBEAT_INTERVAL_DOWN_PHASE3: Duration = Duration::from_secs(60); // 30+ minutes

// Phase durations for adaptive logging
const PHASE1_DURATION: Duration = Duration::from_secs(5 * 60); // 5 minutes
const PHASE2_DURATION: Duration = Duration::from_secs(30 * 60); // 30 minutes

/// Global gateway health tracker singleton
pub static GATEWAY_HEALTH: LazyLock<Arc<RwLock<GatewayHealth>>> =
    LazyLock::new(|| Arc::new(RwLock::new(GatewayHealth::new())));

/// Gateway health state
#[derive(Debug, Clone, PartialEq)]
pub enum HealthState {
    /// All operations succeeding normally
    Healthy,

    /// Some operations failing, some succeeding (intermittent issues)
    Degraded { failure_rate: f32 },

    /// All or most operations failing (gateway down or unreachable)
    Down,
}

/// Gateway health tracker
pub struct GatewayHealth {
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

impl Default for GatewayHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayHealth {
    pub fn new() -> Self {
        Self {
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
        tracing::warn!("🟡 Gateway experiencing intermittent errors");
    }

    fn transition_degraded_to_down(&mut self) {
        self.state = HealthState::Down;
        self.last_state_change = Instant::now();
        self.recovery_attempts = 0;
        tracing::info!("🔴 Gateway connection lost - retrying...");
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
        self.state = HealthState::Degraded { failure_rate: self.failure_rate() };
        self.recovery_attempts = 1;
        self.last_state_change = Instant::now();
        tracing::info!("🟡 Gateway partially restored - monitoring stability...");
    }

    fn try_transition_to_healthy(&mut self) {
        self.recovery_attempts += 1;

        if self.should_transition_to_healthy() {
            let downtime = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(0));
            let failed_ops = self.failed_requests;

            self.state = HealthState::Healthy;
            self.last_state_change = Instant::now();

            tracing::info!(
                "🟢 Gateway UP - Restored after {} ({} operations failed during outage)",
                format_duration(downtime),
                failed_ops
            );

            self.reset_metrics();
        }
    }

    fn should_transition_to_healthy(&self) -> bool {
        self.consecutive_successes >= CONSECUTIVE_SUCCESSES_FOR_RECOVERY
            || (self.recovery_attempts >= RECOVERY_ATTEMPTS_THRESHOLD
                && self.failure_rate() < FAILURE_RATE_HEALTHY_THRESHOLD)
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
        let elapsed = self.first_failure_time.unwrap().elapsed();

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

                tracing::warn!(
                    "🟡 Gateway unstable ({}) - {:.0}% failure rate, operations affected: {}",
                    format_duration(duration),
                    failure_rate * 100.0,
                    affected_ops.join(", ")
                );
            }

            HealthState::Down => {
                let duration = self.first_failure_time.map(|t| t.elapsed()).unwrap_or(Duration::from_secs(0));
                let phase = get_retry_phase(duration);

                tracing::info!(
                    "🔴 Gateway down ({}) - Phase: {} → {} failed operations",
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
}

/// Start the background health monitor task
///
/// This should be called once at application startup.
/// It spawns a tokio task that periodically logs gateway health status.
/// Safe to call multiple times - only starts once.
pub fn start_gateway_health_monitor() {
    use std::sync::Once;
    static START: Once = Once::new();

    START.call_once(|| {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;

                let mut health = GATEWAY_HEALTH.write().await;

                if health.should_log_heartbeat() {
                    health.log_status();
                }
            }
        });

        tracing::debug!("Gateway health monitor started");
    });
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
        let mut health = GatewayHealth::new();

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

        // First success -> Degraded (recovery)
        health.report_success();
        assert!(matches!(health.state, HealthState::Degraded { .. }));

        // More successes -> Healthy
        health.report_success();
        health.report_success();
        assert!(matches!(health.state, HealthState::Healthy));
    }

    #[test]
    fn test_failure_rate() {
        let mut health = GatewayHealth::new();

        health.report_success();
        health.report_failure("test");
        assert!((health.failure_rate() - 0.5).abs() < 0.01);

        health.report_success();
        assert!((health.failure_rate() - 0.33).abs() < 0.02);
    }
}
