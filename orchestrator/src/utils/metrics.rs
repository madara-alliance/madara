use once_cell::sync::Lazy;
use opentelemetry::metrics::{Counter, Histogram, Meter, Gauge};
use opentelemetry::global;

pub static ORCHESTRATOR_METRICS: Lazy<OrchestratorMetrics> = Lazy::new(|| OrchestratorMetrics::register());

pub struct OrchestratorMetrics {
    pub block_gauge: Gauge<f64>,
    pub successful_job_operations: Counter<u64>,
    pub failed_job_operations: Counter<u64>,
    pub failed_jobs: Counter<u64>,
    pub verification_time: Histogram<f64>,
    pub jobs_response_time: Histogram<f64>,
    pub db_calls_response_time: Histogram<f64>,
}

impl OrchestratorMetrics {
    pub fn register() -> Self {
        // Create a meter (the API changed from 0.25.x)
        let meter: Meter = global::meter("crates.orchestrator.opentelemetry");

        // --- Instruments ---
        let block_gauge = meter
            .f64_gauge("block_state")
            .with_description("A gauge to show block state at given time")
            .with_unit("block")
            .build();

        let successful_job_operations = meter
            .u64_counter("successful_job_operations")
            .with_description("Count of successful job operations over time")
            .with_unit("jobs")
            .build();

        let failed_job_operations = meter
            .u64_counter("failed_job_operations")
            .with_description("Count of failed job operations over time")
            .with_unit("jobs")
            .build();

        let failed_jobs = meter
            .u64_counter("failed_jobs")
            .with_description("Count of failed jobs over time")
            .with_unit("jobs")
            .build();

        let verification_time = meter
            .f64_histogram("verification_time")
            .with_description("Time taken for verification of tasks")
            .with_unit("ms")
            .build();

        let jobs_response_time = meter
            .f64_histogram("jobs_response_time")
            .with_description("Response time of jobs over time")
            .with_unit("s")
            .build();

        let db_calls_response_time = meter
            .f64_histogram("db_calls_response_time")
            .with_description("Response time of DB calls over time")
            .with_unit("s")
            .build();

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
