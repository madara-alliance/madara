use crate::error::other::OtherError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum StateUpdateError {
    #[error("Block numbers list should not be empty.")]
    EmptyBlockNumberList,

    #[error("Could not find current attempt number.")]
    AttemptNumberNotFound,

    #[error("last_failed_block should be a positive number")]
    LastFailedBlockNonPositive,

    #[error("Block numbers to settle must be specified (state update job #{internal_id:?})")]
    UnspecifiedBlockNumber { internal_id: String },

    #[error("Could not find tx hashes metadata for the current attempt")]
    TxnHashMetadataNotFound,

    #[error("Tx {tx_hash:?} should not be pending.")]
    TxnShouldNotBePending { tx_hash: String },

    #[error(
        "Last number in block_numbers array returned as None. Possible Error : Delay in job processing or Failed job \
         execution."
    )]
    LastNumberReturnedError,

    #[error("No block numbers found.")]
    BlockNumberNotFound,

    #[error("Duplicated block numbers.")]
    DuplicateBlockNumbers,

    #[error("Block numbers aren't sorted in increasing order.")]
    UnsortedBlockNumbers,

    #[error("Gap detected between the first block to settle and the last one settled.")]
    GapBetweenFirstAndLastBlock,

    #[error("Block #{block_no:?} - SNOS error, [use_kzg_da] should be either 0 or 1.")]
    UseKZGDaError { block_no: u64 },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}
