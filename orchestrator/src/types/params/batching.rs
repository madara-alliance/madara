use crate::cli::batching::BatchingCliArgs;
use crate::types::constant::BLOB_LEN;

#[derive(Debug, Clone)]
pub struct BatchingParams {
    pub max_batch_time_seconds: u64,
    pub max_batch_size: u64,
    pub batching_worker_lock_duration: u64,
    pub max_blocks_per_snos_batch: u64,
    pub fixed_blocks_per_snos_batch: Option<u64>,
    pub max_snos_batches_per_aggregator_batch: u64,
    pub max_num_blobs: usize,
    /// Max number of felts (each encoded using 256 bits) that can be attached in a single state
    /// update transaction.
    ///
    /// It's calculated by multiplying [BLOB_LEN] with max number of blobs to attach in a txn
    /// (taken through ENV/CLI, default is 6, max that ethereum allows is 9)
    pub max_blob_size: usize,
    /// Default proving gas to use for empty blocks.
    /// Empty blocks return zero proving_gas from the bouncer weights API, but every block
    /// has some proving cost (~4,775 steps = ~477,500 gas). We use 1.5M gas (~3x safety margin).
    /// This value is used when proving_gas is zero.
    pub default_empty_block_proving_gas: u64,
}

impl From<BatchingCliArgs> for BatchingParams {
    fn from(args: BatchingCliArgs) -> Self {
        Self {
            max_batch_time_seconds: args.max_batch_time_seconds,
            max_batch_size: args.max_batch_size,
            batching_worker_lock_duration: args.batching_worker_lock_duration,
            max_blocks_per_snos_batch: args.max_blocks_per_snos_batch,
            fixed_blocks_per_snos_batch: args.fixed_blocks_per_snos_batch,
            max_snos_batches_per_aggregator_batch: args.max_snos_batches_per_aggregator_batch,
            max_num_blobs: args.max_num_blobs,
            max_blob_size: args.max_num_blobs * BLOB_LEN,
            default_empty_block_proving_gas: args.default_empty_block_proving_gas,
        }
    }
}
