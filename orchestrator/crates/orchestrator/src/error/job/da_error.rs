use crate::error::other::OtherError;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug, PartialEq)]
pub enum DaError {
    #[error("Cannot process block {block_no:?} for job id {job_id:?} as it's still in pending state.")]
    BlockPending { block_no: String, job_id: Uuid },

    #[error("Blob size must be at least 32 bytes to accommodate a single FieldElement/BigUint, but was {blob_size:?}")]
    InsufficientBlobSize { blob_size: u64 },

    #[error(
        "Exceeded the maximum number of blobs per transaction: allowed {max_blob_per_txn:?}, found \
         {current_blob_length:?} for block {block_no:?} and job id {job_id:?}"
    )]
    MaxBlobsLimitExceeded { max_blob_per_txn: u64, current_blob_length: u64, block_no: String, job_id: Uuid },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}
