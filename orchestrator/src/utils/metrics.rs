use crate::core::client::database::constant::JOBS_COLLECTION;
use crate::utils::job_status_metrics::JobStatusTracker;
use once_cell;
use once_cell::sync::Lazy;
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, KeyValue};
use orchestrator_utils::metrics::lib::{register_counter_metric_instrument, register_gauge_metric_instrument, Metrics};
use orchestrator_utils::register_metric;

register_metric!(ORCHESTRATOR_METRICS, OrchestratorMetrics);

pub struct OrchestratorMetrics {
    pub block_gauge: Gauge<f64>,
    pub successful_job_operations: Counter<f64>,
    pub failed_job_operations: Counter<f64>,
    pub failed_jobs: Counter<f64>,
    pub verification_time: Gauge<f64>,
    pub jobs_response_time: Gauge<f64>,
    pub db_calls_response_time: Gauge<f64>,
    // Queue Metrics
    pub job_queue_depth: Gauge<f64>,
    pub job_queue_wait_time: Gauge<f64>,
    pub job_scheduling_delay: Gauge<f64>,
    // Processing Pipeline Metrics
    pub job_retry_count: Counter<f64>,
    pub job_state_transitions: Counter<f64>,
    pub job_timeout_count: Counter<f64>,
    pub job_abandoned_count: Counter<f64>,
    // Latency Metrics
    pub job_e2e_latency: Gauge<f64>,
    pub proof_generation_time: Gauge<f64>,
    pub settlement_time: Gauge<f64>,
    // Throughput Metrics
    pub jobs_per_minute: Gauge<f64>,
    pub blocks_per_hour: Gauge<f64>,
    // Dependency Metrics
    pub dependency_wait_time: Gauge<f64>,
    pub orphaned_jobs: Counter<f64>,
    // Resource Metrics
    pub job_parallelism_factor: Gauge<f64>,
    pub active_jobs_count: Gauge<f64>,
    // SLA Metrics
    pub sla_breach_count: Counter<f64>,
    pub job_age_p99: Gauge<f64>,
    pub batching_rate: Gauge<f64>,
    pub batch_creation_time: Gauge<f64>,
    // Job Status Tracking
    pub job_status_tracker: JobStatusTracker,
}

impl Metrics for OrchestratorMetrics {
    fn register() -> Self {
        // Register meter
        let common_scope_attributes = vec![KeyValue::new("crate", "orchestrator")];
        let orchestrator_meter = global::meter_with_version(
            "crates.orchestrator.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        // Register all instruments
        let block_gauge = register_gauge_metric_instrument(
            &orchestrator_meter,
            "block_state".to_string(),
            "A gauge to show block state at given time".to_string(),
            "block".to_string(),
        );

        let successful_job_operations = register_counter_metric_instrument(
            &orchestrator_meter,
            "successful_job_operations".to_string(),
            "A counter to show count of successful job operations over time".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let failed_job_operations = register_counter_metric_instrument(
            &orchestrator_meter,
            "failed_job_operations".to_string(),
            "A counter to show count of failed job operations over time".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let failed_jobs = register_counter_metric_instrument(
            &orchestrator_meter,
            "failed_jobs".to_string(),
            "A counter to show count of failed jobs over time".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let verification_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "verification_time".to_string(),
            "A gauge to show the time taken for verification of tasks".to_string(),
            "ms".to_string(),
        );

        let jobs_response_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "jobs_response_time".to_string(),
            "A gauge to show response time of jobs over time".to_string(),
            "s".to_string(),
        );

        let db_calls_response_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "db_calls_response_time".to_string(),
            "A gauge to show response time of jobs over time".to_string(),
            "s".to_string(),
        );

        // Queue Metrics
        let job_queue_depth = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_queue_depth".to_string(),
            "Number of jobs waiting in queue".to_string(),
            "jobs".to_string(),
        );

        let job_queue_wait_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_queue_wait_time".to_string(),
            "Time spent waiting in queue".to_string(),
            "s".to_string(),
        );

        let job_scheduling_delay = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_scheduling_delay".to_string(),
            "Delay between job creation and pickup".to_string(),
            "s".to_string(),
        );

        // Processing Pipeline Metrics
        let job_retry_count = register_counter_metric_instrument(
            &orchestrator_meter,
            "job_retry_count".to_string(),
            "Number of job retries".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let job_state_transitions = register_counter_metric_instrument(
            &orchestrator_meter,
            "job_state_transitions".to_string(),
            "Count of job state transitions".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let job_timeout_count = register_counter_metric_instrument(
            &orchestrator_meter,
            "job_timeout_count".to_string(),
            "Number of jobs that timed out".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let job_abandoned_count = register_counter_metric_instrument(
            &orchestrator_meter,
            "job_abandoned_count".to_string(),
            "Jobs abandoned after max retries".to_string(),
            String::from(JOBS_COLLECTION),
        );

        // Latency Metrics
        let job_e2e_latency = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_e2e_latency".to_string(),
            "End-to-end job processing time".to_string(),
            "s".to_string(),
        );

        let proof_generation_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "proof_generation_time".to_string(),
            "Time to generate proofs".to_string(),
            "s".to_string(),
        );

        let settlement_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "settlement_time".to_string(),
            "Time to settle on L1/L2".to_string(),
            "s".to_string(),
        );

        // Throughput Metrics
        let jobs_per_minute = register_gauge_metric_instrument(
            &orchestrator_meter,
            "jobs_per_minute".to_string(),
            "Jobs processed per minute".to_string(),
            "jobs/min".to_string(),
        );

        let blocks_per_hour = register_gauge_metric_instrument(
            &orchestrator_meter,
            "blocks_per_hour".to_string(),
            "Blocks processed per hour".to_string(),
            "blocks/hour".to_string(),
        );

        // Dependency Metrics
        let dependency_wait_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "dependency_wait_time".to_string(),
            "Time waiting for dependencies".to_string(),
            "s".to_string(),
        );

        let orphaned_jobs = register_counter_metric_instrument(
            &orchestrator_meter,
            "orphaned_jobs".to_string(),
            "Jobs with missing dependencies".to_string(),
            String::from(JOBS_COLLECTION),
        );

        // Resource Metrics
        let job_parallelism_factor = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_parallelism_factor".to_string(),
            "Number of parallel jobs running".to_string(),
            "jobs".to_string(),
        );

        let active_jobs_count = register_gauge_metric_instrument(
            &orchestrator_meter,
            "active_jobs_count".to_string(),
            "Currently active jobs".to_string(),
            "jobs".to_string(),
        );

        // SLA Metrics
        let sla_breach_count = register_counter_metric_instrument(
            &orchestrator_meter,
            "sla_breach_count".to_string(),
            "Number of SLA breaches".to_string(),
            String::from(JOBS_COLLECTION),
        );

        let job_age_p99 = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_age_p99".to_string(),
            "99th percentile of job age".to_string(),
            "s".to_string(),
        );

        // Batching Metrics
        let batching_rate = register_gauge_metric_instrument(
            &orchestrator_meter,
            "batching_rate".to_string(),
            "Number of batches created per hour".to_string(),
            "batches/hour".to_string(),
        );

        let batch_creation_time = register_gauge_metric_instrument(
            &orchestrator_meter,
            "batch_creation_time".to_string(),
            "Average time to create a batch".to_string(),
            "s".to_string(),
        );

        // Job Status Tracking Metrics
        let job_status_gauge = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_status_current".to_string(),
            "Current status of jobs by block, type, and status".to_string(),
            "status".to_string(),
        );

        let job_transition_gauge = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_status_transitions".to_string(),
            "Job status transitions".to_string(),
            "transitions".to_string(),
        );

        let job_details_gauge = register_gauge_metric_instrument(
            &orchestrator_meter,
            "job_details".to_string(),
            "Detailed job information with job_id and status".to_string(),
            "jobs".to_string(),
        );

        let job_status_tracker = JobStatusTracker { job_status_gauge, job_transition_gauge, job_details_gauge };

        Self {
            block_gauge,
            successful_job_operations,
            failed_job_operations,
            failed_jobs,
            verification_time,
            jobs_response_time,
            db_calls_response_time,
            job_queue_depth,
            job_queue_wait_time,
            job_scheduling_delay,
            job_retry_count,
            job_state_transitions,
            job_timeout_count,
            job_abandoned_count,
            job_e2e_latency,
            proof_generation_time,
            settlement_time,
            jobs_per_minute,
            blocks_per_hour,
            dependency_wait_time,
            orphaned_jobs,
            job_parallelism_factor,
            active_jobs_count,
            sla_breach_count,
            job_age_p99,
            batching_rate,
            batch_creation_time,
            job_status_tracker,
        }
    }
}
