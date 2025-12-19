/// Configuration for greedy worker behavior
///
/// FIX-08: Configurable graceful shutdown timeouts per job type
/// FIX-05: Exponential backoff configuration for database errors
/// FIX-07: Circuit breaker thresholds
use crate::types::jobs::types::JobType;
use std::collections::HashMap;
use std::time::Duration;

/// Configuration for a single greedy worker
#[derive(Debug, Clone)]
pub struct GreedyWorkerConfig {
    /// Job type this worker handles
    pub job_type: JobType,

    /// Polling interval when no jobs are available (milliseconds)
    pub poll_interval_ms: u64,

    /// Maximum concurrent jobs of this type (processing + verification)
    pub max_concurrent_jobs: usize,

    /// FIX-08: Graceful shutdown timeout for this job type
    /// Time to wait for in-flight jobs to complete before forceful shutdown
    pub shutdown_timeout: Duration,

    /// FIX-05: Initial backoff delay on database errors (milliseconds)
    pub backoff_initial_ms: u64,

    /// FIX-05: Maximum backoff delay on database errors (milliseconds)
    pub backoff_max_ms: u64,

    /// FIX-05: Backoff multiplier for exponential increase
    pub backoff_multiplier: f64,

    /// FIX-07: Circuit breaker error threshold
    /// Number of consecutive errors before circuit breaker opens
    pub circuit_breaker_threshold: u32,

    /// FIX-07: Circuit breaker reset timeout (seconds)
    /// Time to wait before attempting to close the circuit breaker
    pub circuit_breaker_reset_timeout_secs: u64,
}

impl GreedyWorkerConfig {
    /// Create a new greedy worker configuration with default values
    pub fn new(job_type: JobType, poll_interval_ms: u64) -> Self {
        let shutdown_timeout = Self::default_shutdown_timeout(&job_type);
        Self {
            job_type,
            poll_interval_ms,
            max_concurrent_jobs: 10, // Default concurrency limit
            shutdown_timeout,
            backoff_initial_ms: 100,
            backoff_max_ms: 30_000, // 30 seconds max
            backoff_multiplier: 2.0,
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_timeout_secs: 60,
        }
    }

    /// FIX-08: Default shutdown timeout based on job type complexity
    fn default_shutdown_timeout(job_type: &JobType) -> Duration {
        match job_type {
            // Long-running proof jobs need more time
            JobType::ProofCreation | JobType::ProofRegistration => Duration::from_secs(300), // 5 minutes
            // SNOS jobs are moderately complex
            JobType::SnosRun => Duration::from_secs(180), // 3 minutes
            // State updates and data submission are quicker
            JobType::StateTransition | JobType::DataSubmission => Duration::from_secs(120), // 2 minutes
            // Default for other job types
            _ => Duration::from_secs(60), // 1 minute
        }
    }

    /// Set maximum concurrent jobs
    pub fn with_max_concurrent_jobs(mut self, max: usize) -> Self {
        self.max_concurrent_jobs = max;
        self
    }

    /// Set custom shutdown timeout
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Set backoff configuration
    pub fn with_backoff(mut self, initial_ms: u64, max_ms: u64, multiplier: f64) -> Self {
        self.backoff_initial_ms = initial_ms;
        self.backoff_max_ms = max_ms;
        self.backoff_multiplier = multiplier;
        self
    }

    /// Set circuit breaker configuration
    pub fn with_circuit_breaker(mut self, threshold: u32, reset_timeout_secs: u64) -> Self {
        self.circuit_breaker_threshold = threshold;
        self.circuit_breaker_reset_timeout_secs = reset_timeout_secs;
        self
    }

    /// Calculate next backoff delay using exponential backoff
    pub fn calculate_backoff(&self, attempt: u32) -> Duration {
        let delay_ms = (self.backoff_initial_ms as f64 * self.backoff_multiplier.powi(attempt as i32))
            .min(self.backoff_max_ms as f64) as u64;
        Duration::from_millis(delay_ms)
    }
}

/// Configuration for all greedy workers
#[derive(Debug, Clone)]
pub struct GreedyWorkersConfig {
    /// Per-job-type worker configurations
    pub workers: HashMap<JobType, GreedyWorkerConfig>,

    /// Global orchestrator instance ID for claim tracking
    pub orchestrator_id: String,

    /// Global poll interval (used as default)
    pub default_poll_interval_ms: u64,
}

impl GreedyWorkersConfig {
    /// Create a new greedy workers configuration
    pub fn new(orchestrator_id: String, default_poll_interval_ms: u64) -> Self {
        Self { workers: HashMap::new(), orchestrator_id, default_poll_interval_ms }
    }

    /// Add a worker configuration for a job type
    pub fn add_worker(&mut self, config: GreedyWorkerConfig) {
        self.workers.insert(config.job_type.clone(), config);
    }

    /// Get configuration for a specific job type
    pub fn get_worker_config(&self, job_type: &JobType) -> Option<&GreedyWorkerConfig> {
        self.workers.get(job_type)
    }

    /// Create default configuration for all job types
    pub fn with_all_job_types(mut self) -> Self {
        let job_types = vec![
            JobType::DataSubmission,
            JobType::StateTransition,
            JobType::SnosRun,
            JobType::ProofCreation,
            JobType::ProofRegistration,
        ];

        for job_type in job_types {
            self.add_worker(GreedyWorkerConfig::new(job_type, self.default_poll_interval_ms));
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        let config = GreedyWorkerConfig::new(JobType::DataSubmission, 100);

        assert_eq!(config.calculate_backoff(0), Duration::from_millis(100));
        assert_eq!(config.calculate_backoff(1), Duration::from_millis(200));
        assert_eq!(config.calculate_backoff(2), Duration::from_millis(400));
        assert_eq!(config.calculate_backoff(3), Duration::from_millis(800));
    }

    #[test]
    fn test_backoff_max_limit() {
        let config = GreedyWorkerConfig::new(JobType::DataSubmission, 100).with_backoff(1000, 5000, 2.0);

        assert_eq!(config.calculate_backoff(0), Duration::from_millis(1000));
        assert_eq!(config.calculate_backoff(1), Duration::from_millis(2000));
        assert_eq!(config.calculate_backoff(2), Duration::from_millis(4000));
        assert_eq!(config.calculate_backoff(3), Duration::from_millis(5000)); // Capped at max
        assert_eq!(config.calculate_backoff(10), Duration::from_millis(5000)); // Still capped
    }

    #[test]
    fn test_default_shutdown_timeouts() {
        assert_eq!(GreedyWorkerConfig::new(JobType::ProofCreation, 100).shutdown_timeout, Duration::from_secs(300));
        assert_eq!(GreedyWorkerConfig::new(JobType::DataSubmission, 100).shutdown_timeout, Duration::from_secs(120));
    }
}
