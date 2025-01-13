use once_cell;
use once_cell::sync::Lazy;
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, KeyValue};
use utils::metrics::lib::{register_counter_metric_instrument, register_gauge_metric_instrument, Metrics};
use utils::register_metric;

register_metric!(ORCHESTRATOR_METRICS, OrchestratorMetrics);

pub struct OrchestratorMetrics {
    pub block_gauge: Gauge<f64>,
    pub successful_job_operations: Counter<f64>,
    pub failed_job_operations: Counter<f64>,
    pub failed_jobs: Counter<f64>,
    pub verification_time: Gauge<f64>,
    pub jobs_response_time: Gauge<f64>,
    pub db_calls_response_time: Gauge<f64>,
}

impl Metrics for OrchestratorMetrics {
    fn register() -> Self {
        // Register meter
        let common_scope_attributes = vec![KeyValue::new("crate", "orchestrator")];
        let orchestrator_meter = global::meter_with_version(
            "crates.orchestrator.opentelemetry",
            // TODO: Unsure of these settings, come back
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
            "jobs".to_string(),
        );

        let failed_job_operations = register_counter_metric_instrument(
            &orchestrator_meter,
            "failed_job_operations".to_string(),
            "A counter to show count of failed job operations over time".to_string(),
            "jobs".to_string(),
        );

        let failed_jobs = register_counter_metric_instrument(
            &orchestrator_meter,
            "failed_jobs".to_string(),
            "A counter to show count of failed jobs over time".to_string(),
            "jobs".to_string(),
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

        Self {
            block_gauge,
            successful_job_operations,
            failed_job_operations,
            failed_jobs,
            verification_time,
            jobs_response_time,
            db_calls_response_time,
        }
    }
}
