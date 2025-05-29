use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct ServiceCliArgs {
    /// The maximum block to process. defaults to i64::MAX (and not u64, since we use it as i64 later on)
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", long, default_value = "9223372036854775807")]
    pub max_block_to_process: u64,

    /// The minimum block to process.
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
}
