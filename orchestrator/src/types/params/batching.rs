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
    /// Buffer (in number of felts) subtracted from the maximum blob capacity to account for
    /// overhead added by the prover (e.g. encryption headers).
    pub blob_size_buffer: usize,
    /// Max number of felts (each encoded using 256 bits) that can be attached in a single state
    /// update transaction.
    ///
    /// It's calculated as (max_num_blobs * [BLOB_LEN]) - blob_size_buffer
    pub max_blob_size: usize,
    /// Default proving gas to use for empty blocks.
    /// Empty blocks return zero proving_gas from the bouncer weights API, but every block
    /// has some proving cost (~4,775 steps = ~477,500 gas). We use 1.5M gas (~3x safety margin).
    /// This value is used when proving_gas is zero.
    pub default_empty_block_proving_gas: u64,
    pub max_batch_processing_size: u64,
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
            blob_size_buffer: args.blob_size_buffer,
            max_blob_size: args.max_num_blobs * BLOB_LEN - args.blob_size_buffer,
            default_empty_block_proving_gas: args.default_empty_block_proving_gas,
            max_batch_processing_size: args.max_batch_processing_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cli_args(max_num_blobs: usize, blob_size_buffer: usize) -> BatchingCliArgs {
        BatchingCliArgs {
            max_batch_time_seconds: 3600,
            max_batch_size: 100,
            max_num_blobs,
            blob_size_buffer,
            batching_worker_lock_duration: 3600,
            max_blocks_per_snos_batch: 10,
            fixed_blocks_per_snos_batch: None,
            max_snos_batches_per_aggregator_batch: 50,
            default_empty_block_proving_gas: 1500000,
            max_batch_processing_size: 10,
        }
    }

    #[test]
    fn test_max_blob_size_with_default_buffer() {
        let params = BatchingParams::from(make_cli_args(6, 15));
        assert_eq!(params.max_blob_size, 6 * BLOB_LEN - 15);
        assert_eq!(params.max_blob_size, 24561);
    }

    #[test]
    fn test_max_blob_size_with_zero_buffer() {
        let params = BatchingParams::from(make_cli_args(6, 0));
        assert_eq!(params.max_blob_size, 6 * BLOB_LEN);
        assert_eq!(params.max_blob_size, 24576);
    }

    #[test]
    fn test_max_blob_size_with_custom_blobs_and_buffer() {
        let params = BatchingParams::from(make_cli_args(9, 100));
        assert_eq!(params.max_blob_size, 9 * BLOB_LEN - 100);
        assert_eq!(params.max_blob_size, 36764);
    }
}
