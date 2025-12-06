use clap::Args;

/// Parameters used to configure the batching.
#[derive(Debug, Clone, Args)]
pub struct BatchingCliArgs {
    /// Max batch time in seconds.
    /// Optional when using --config or --preset (used as override)
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BATCH_TIME_SECONDS", long)]
    pub max_batch_time_seconds: Option<u64>,

    /// Max batch size.
    /// Optional when using --config or --preset (used as override)
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BATCH_SIZE", long)]
    pub max_batch_size: Option<u64>,

    /// Max number of blobs to attach in a single state update transaction
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_NUM_BLOBS", long, default_value = "6")]
    pub max_num_blobs: usize,

    /// Batching worker lock duration in seconds.
    /// This lock ensures that only one Batching Worker is running at a time.
    #[arg(env = "MADARA_ORCHESTRATOR_BATCHING_LOCK_DURATION_SECONDS", long, default_value = "3600")]
    pub batching_worker_lock_duration: u64,

    /// Maximum number of blocks allowed in a single SNOS batch.
    /// Keep this None if you don't want to specify a hard limit.
    /// This can be used to test with RPC other than Madara.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_BLOCKS_PER_SNOS_BATCH", long)]
    pub max_blocks_per_snos_batch: Option<u64>,

    /// Maximum number of SNOS batches that can be children of one aggregator batch.
    #[arg(env = "MADARA_ORCHESTRATOR_MAX_SNOS_BATCHES_PER_AGGREGATOR_BATCH", long, default_value = "50")]
    pub max_snos_batches_per_aggregator_batch: u64,

    /// Default proving gas to use for empty blocks.
    /// Empty blocks return zero proving_gas from the bouncer weights API, but every block
    /// has some proving cost (~4,775 steps = ~477,500 gas). We use 1.5M gas (~3x safety margin).
    /// This value is used when proving_gas is zero.
    #[arg(env = "MADARA_ORCHESTRATOR_DEFAULT_EMPTY_BLOCK_PROVING_GAS", long, default_value = "1500000")]
    pub default_empty_block_proving_gas: u64,
}
