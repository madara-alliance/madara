use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct ServiceCliArgs {
    /// The maximum block to process.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", long)]
    pub max_block_to_process: Option<u64>,

    /// The minimum block to process.
    #[arg(env = "MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS", long)]
    pub min_block_to_process: Option<u64>,

    /// The maximum number of SNOS jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS", long)]
    pub max_concurrent_snos_jobs: Option<usize>,

    /// The maximum number of proving jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS", long)]
    pub max_concurrent_proving_jobs: Option<usize>,

    /// The maximum number of proof registration jobs to process concurrently.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROOF_REGISTRATION_JOBS", long)]
    pub max_concurrent_proof_registration_jobs: Option<usize>,
}
