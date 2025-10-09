use crate::cli::batching::BatchingCliArgs;
use crate::types::constant::BLOB_LEN;

#[derive(Debug, Clone)]
pub struct BatchingParams {
    pub max_batch_time_seconds: u64,
    pub max_batch_size: u64,
    pub batching_worker_lock_duration: u64,
    pub max_num_blobs: usize,
    /// Max number of felts (each encoded using 256 bits) that can be attached in a single state
    /// update transaction.
    ///
    /// It's calculated by multiplying [BLOB_LEN] with max number of blobs to attach in a txn
    /// (taken through ENV/CLI, default is 6, max that ethereum allows is 9)
    pub max_blob_size: usize,
}

impl From<BatchingCliArgs> for BatchingParams {
    fn from(args: BatchingCliArgs) -> Self {
        Self {
            max_batch_time_seconds: args.max_batch_time_seconds,
            max_batch_size: args.max_batch_size,
            batching_worker_lock_duration: args.batching_worker_lock_duration,
            max_num_blobs: args.max_num_blobs,
            max_blob_size: args.max_num_blobs * BLOB_LEN,
        }
    }
}
