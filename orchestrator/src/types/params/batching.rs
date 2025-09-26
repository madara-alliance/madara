use crate::cli::batching::BatchingCliArgs;

#[derive(Debug, Clone)]
pub struct BatchingParams {
    pub max_batch_time_seconds: u64,
    pub max_batch_size: u64,
    pub batching_worker_lock_duration: u64,
    pub max_blocks_per_snos_batch: Option<u64>,
    pub max_snos_batches_per_aggregator_batch: u64,
}

impl From<BatchingCliArgs> for BatchingParams {
    fn from(args: BatchingCliArgs) -> Self {
        Self {
            max_batch_time_seconds: args.max_batch_time_seconds,
            max_batch_size: args.max_batch_size,
            batching_worker_lock_duration: args.batching_worker_lock_duration,
            max_blocks_per_snos_batch: args.max_blocks_per_snos_batch,
            max_snos_batches_per_aggregator_batch: args.max_snos_batches_per_aggregator_batch,
        }
    }
}
