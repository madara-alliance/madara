/// FIX-09: Greedy-specific metrics for monitoring queue-less architecture
///
/// These metrics provide visibility into greedy worker behavior and performance,
/// helping to identify issues like polling inefficiency, claim contention, and
/// database bottlenecks.
use crate::types::jobs::types::JobType;
use opentelemetry::KeyValue;
use std::sync::LazyLock;
use tracing::info;

/// Metric names for greedy worker operations
pub struct GreedyMetrics {
    /// Counter for successful job claims (processing)
    pub claims_processing_success: &'static str,
    /// Counter for failed job claims (processing)
    pub claims_processing_failed: &'static str,
    /// Counter for successful job claims (verification)
    pub claims_verification_success: &'static str,
    /// Counter for failed job claims (verification)
    pub claims_verification_failed: &'static str,
    /// Counter for empty poll cycles (no jobs available)
    pub empty_polls: &'static str,
    /// Gauge for currently claimed jobs per orchestrator instance
    pub active_claims: &'static str,
    /// Histogram for job claim latency
    pub claim_latency: &'static str,
    /// Counter for claim releases (job completion)
    pub claim_releases: &'static str,
    /// Counter for database errors during claiming
    pub db_errors: &'static str,
}

pub static GREEDY_METRICS: LazyLock<GreedyMetrics> = LazyLock::new(|| GreedyMetrics {
    claims_processing_success: "greedy.claims.processing.success",
    claims_processing_failed: "greedy.claims.processing.failed",
    claims_verification_success: "greedy.claims.verification.success",
    claims_verification_failed: "greedy.claims.verification.failed",
    empty_polls: "greedy.polls.empty",
    active_claims: "greedy.claims.active",
    claim_latency: "greedy.claim.latency",
    claim_releases: "greedy.claim.releases",
    db_errors: "greedy.db.errors",
});

/// Record a successful processing job claim
pub fn record_processing_claim_success(job_type: &JobType) {
    info!(
        metric = GREEDY_METRICS.claims_processing_success,
        job_type = ?job_type,
        "Processing job claimed successfully"
    );
}

/// Record a failed processing job claim attempt (no jobs available)
pub fn record_processing_claim_failed(job_type: &JobType) {
    info!(
        metric = GREEDY_METRICS.claims_processing_failed,
        job_type = ?job_type,
        "No processing jobs available to claim"
    );
}

/// Record a successful verification job claim
pub fn record_verification_claim_success(job_type: &JobType) {
    info!(
        metric = GREEDY_METRICS.claims_verification_success,
        job_type = ?job_type,
        "Verification job claimed successfully"
    );
}

/// Record a failed verification job claim attempt (no jobs available)
pub fn record_verification_claim_failed(job_type: &JobType) {
    info!(
        metric = GREEDY_METRICS.claims_verification_failed,
        job_type = ?job_type,
        "No verification jobs available to claim"
    );
}

/// Record an empty poll cycle (no jobs available in either phase)
pub fn record_empty_poll(job_type: &JobType) {
    info!(
        metric = GREEDY_METRICS.empty_polls,
        job_type = ?job_type,
        "Poll cycle completed with no jobs available"
    );
}

/// Record current active claims count
pub fn record_active_claims(orchestrator_id: &str, count: u64) {
    info!(
        metric = GREEDY_METRICS.active_claims,
        orchestrator_id = orchestrator_id,
        count = count,
        "Active claims gauge updated"
    );
}

/// Record job claim latency
pub fn record_claim_latency(job_type: &JobType, phase: &str, duration_ms: f64) {
    info!(
        metric = GREEDY_METRICS.claim_latency,
        job_type = ?job_type,
        phase = phase,
        duration_ms = duration_ms,
        "Job claim latency recorded"
    );
}

/// Record a claim release (job completion or failure)
pub fn record_claim_release(job_type: &JobType, reason: &str) {
    info!(
        metric = GREEDY_METRICS.claim_releases,
        job_type = ?job_type,
        reason = reason,
        "Job claim released"
    );
}

/// Record a database error during greedy operations
pub fn record_db_error(operation: &str, error: &str) {
    info!(
        metric = GREEDY_METRICS.db_errors,
        operation = operation,
        error = error,
        "Database error during greedy operation"
    );
}

/// Helper function to create attributes for metrics
pub fn job_type_attributes(job_type: &JobType) -> [KeyValue; 1] {
    [KeyValue::new("job_type", format!("{:?}", job_type))]
}

/// Helper function to create attributes with phase information
pub fn phase_attributes(job_type: &JobType, phase: &str) -> [KeyValue; 2] {
    [KeyValue::new("job_type", format!("{:?}", job_type)), KeyValue::new("phase", phase.to_string())]
}
