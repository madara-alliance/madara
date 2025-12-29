use clap::Args;

fn parse_positive_usize(s: &str) -> Result<usize, String> {
    let value: usize = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if value == 0 {
        return Err("value must be greater than 0".to_string());
    }
    Ok(value)
}

#[derive(Debug, Clone, Args)]
pub struct ServiceCliArgs {
    /// The maximum block to process.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", long)]
    pub max_block_to_process: Option<u64>,

    /// The minimum block to process. defaults to u64::MIN
    #[arg(env = "MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS", long, default_value = "0")]
    pub min_block_to_process: u64,

    /// The maximum number of SNOS jobs to create concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_SNOS_JOBS", long, default_value = "200")]
    pub max_concurrent_created_snos_jobs: u64,

    /// The maximum number of SNOS jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS", long)]
    pub max_concurrent_snos_jobs: Option<usize>,

    /// The maximum number of proving jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS", long)]
    pub max_concurrent_proving_jobs: Option<usize>,

    /// Timeout in seconds for jobs stuck in LockedForProcessing status before self-healing recovery.
    /// This is used as a fallback for job types that don't have a specific timeout configured.
    #[arg(env = "MADARA_ORCHESTRATOR_JOB_PROCESSING_TIMEOUT_SECONDS", long, default_value = "1800")]
    pub job_processing_timeout_seconds: u64,

    /// Timeout in seconds for SNOS jobs stuck in LockedForProcessing status.
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_JOB_TIMEOUT_SECONDS", long, default_value = "3600")]
    pub snos_job_timeout_seconds: u64,

    /// Timeout in seconds for Proving jobs stuck in LockedForProcessing status.
    #[arg(env = "MADARA_ORCHESTRATOR_PROVING_JOB_TIMEOUT_SECONDS", long, default_value = "1800")]
    pub proving_job_timeout_seconds: u64,

    /// Timeout in seconds for Proof Registration jobs stuck in LockedForProcessing status.
    #[arg(env = "MADARA_ORCHESTRATOR_PROOF_REGISTRATION_TIMEOUT_SECONDS", long, default_value = "1800")]
    pub proof_registration_timeout_seconds: u64,

    /// Timeout in seconds for Data Submission jobs stuck in LockedForProcessing status.
    #[arg(env = "MADARA_ORCHESTRATOR_DATA_SUBMISSION_TIMEOUT_SECONDS", long, default_value = "1800")]
    pub data_submission_timeout_seconds: u64,

    /// Timeout in seconds for State Transition jobs stuck in LockedForProcessing status.
    #[arg(env = "MADARA_ORCHESTRATOR_STATE_TRANSITION_TIMEOUT_SECONDS", long, default_value = "2700")]
    pub state_transition_timeout_seconds: u64,

    /// Target number of SNOS jobs to maintain in the processing pipeline/buffer.
    /// New jobs are created when the buffer drops below this size.
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_JOB_BUFFER_SIZE", long, default_value = "50")]
    pub snos_job_buffer_size: u64,

    /// Maximum number of messages allowed in the priority queue. Must be greater than 0.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_PRIORITY_QUEUE_SIZE", long, default_value = "20", value_parser = parse_positive_usize)]
    pub max_priority_queue_size: usize,
}
