use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct ServiceCliArgs {
    /// The maximum block to process.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", long)]
    pub max_block_to_process: Option<u64>,

    /// The minimum block to process. defaults to u64::MIN
    #[arg(env = "MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS", long, default_value = "0")]
    pub min_block_to_process: u64,

    /// The maximum number of SNOS jobs in Created+PendingRetry status (caps job creation).
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_SNOS_JOBS", long, default_value = "200")]
    pub max_concurrent_created_snos_jobs: u64,

    /// The maximum number of ProofCreation jobs in Created+PendingRetry status (caps job creation).
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_PROVING_JOBS", long, default_value = "200")]
    pub max_concurrent_created_proving_jobs: u64,

    /// The maximum number of Aggregator jobs in Created+PendingRetry status (caps job creation).
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_AGGREGATOR_JOBS", long, default_value = "50")]
    pub max_concurrent_created_aggregator_jobs: u64,

    /// The maximum number of SNOS jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS", long)]
    pub max_concurrent_snos_jobs: Option<usize>,

    /// The maximum number of proving jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS", long)]
    pub max_concurrent_proving_jobs: Option<usize>,

    /// Timeout in seconds for jobs stuck in LockedForProcessing status before self-healing recovery.
    #[arg(env = "MADARA_ORCHESTRATOR_JOB_PROCESSING_TIMEOUT_SECONDS", long, default_value = "1800")]
    pub job_processing_timeout_seconds: u64,

    /// Enable queue-less greedy worker mode (disables SQS)
    #[arg(env = "MADARA_ORCHESTRATOR_GREEDY_MODE", long, default_value = "false")]
    pub greedy_mode: bool,

    /// Polling interval in milliseconds when no jobs are available in greedy mode
    #[arg(env = "MADARA_ORCHESTRATOR_GREEDY_POLL_INTERVAL_MS", long, default_value = "100")]
    pub greedy_poll_interval_ms: u64,
}
