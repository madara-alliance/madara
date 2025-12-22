use clap::Args;

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
    #[arg(env = "MADARA_ORCHESTRATOR_JOB_PROCESSING_TIMEOUT_SECONDS", long, default_value = "1800")]
    pub job_processing_timeout_seconds: u64,

    /// Target number of SNOS jobs to maintain in the processing pipeline/buffer.
    /// New jobs are created when the buffer drops below this size.
    #[arg(env = "MADARA_ORCHESTRATOR_SNOS_JOB_BUFFER_SIZE", long, default_value = "50")]
    pub snos_job_buffer_size: u64,

    /// Maximum number of messages allowed in the priority queue.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_PRIORITY_QUEUE_SIZE", long, default_value = "20")]
    pub max_priority_queue_size: usize,

    /// Maximum time (in seconds) to wait for the priority slot to become empty.
    #[arg(env = "MADARA_ORCHESTRATOR_PRIORITY_SLOT_WAIT_TIMEOUT", long, default_value = "300")]
    pub priority_slot_wait_timeout_secs: u64,

    /// Maximum time (in seconds) a message can sit in the priority slot before being
    /// considered stale. Stale messages are NACKed to enable DLQ flow.
    #[arg(env = "MADARA_ORCHESTRATOR_PRIORITY_SLOT_STALENESS_TIMEOUT", long, default_value = "300")]
    pub priority_slot_staleness_timeout_secs: u64,

    /// Interval (in milliseconds) between priority slot availability checks.
    #[arg(env = "MADARA_ORCHESTRATOR_PRIORITY_SLOT_CHECK_INTERVAL_MS", long, default_value = "1000")]
    pub priority_slot_check_interval_ms: u64,
}
