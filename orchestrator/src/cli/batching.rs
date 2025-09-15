use clap::Args;

/// Parameters used to configure the batching.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["max_batch_time_seconds", "max_batch_size"])]
pub struct BatchingCliArgs {
    /// Max batch time in seconds.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BATCH_TIME_SECONDS", long)]
    pub max_batch_time_seconds: u64,

    /// Max batch size.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BATCH_SIZE", long)]
    pub max_batch_size: u64,

    /// Batching worker lock duration in seconds.
    /// This lock ensures that only one Batching Worker is running at a time.
    #[arg(env = "MADARA_ORCHESTRATOR_BATCHING_LOCK_DURATION_SECONDS", long, default_value = "3600")]
    pub batching_worker_lock_duration: u64,
}
