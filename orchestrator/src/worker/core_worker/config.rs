/// Configuration for worker behavior
use crate::types::jobs::types::JobType;
use std::collections::HashMap;

/// Configuration for a single worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub job_type: JobType,
    pub poll_interval_ms: u64,
    pub max_concurrent_processing: usize,
    pub max_concurrent_verification: usize,
}

impl WorkerConfig {
    pub fn new(job_type: JobType, poll_interval_ms: u64) -> Self {
        Self { job_type, poll_interval_ms, max_concurrent_processing: 10, max_concurrent_verification: 5 }
    }

    pub fn with_max_concurrent_processing(mut self, max: usize) -> Self {
        self.max_concurrent_processing = max;
        self
    }

    pub fn with_max_concurrent_verification(mut self, max: usize) -> Self {
        self.max_concurrent_verification = max;
        self
    }
}

/// Configuration for all workers
#[derive(Debug, Clone)]
pub struct WorkersConfig {
    pub workers: HashMap<JobType, WorkerConfig>,
    pub orchestrator_id: String,
}

impl WorkersConfig {
    pub fn new(orchestrator_id: String, _default_poll_interval_ms: u64) -> Self {
        Self { workers: HashMap::new(), orchestrator_id }
    }

    pub fn add_worker(&mut self, config: WorkerConfig) {
        self.workers.insert(config.job_type.clone(), config);
    }
}
