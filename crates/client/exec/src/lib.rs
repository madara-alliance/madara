mod block_context;
mod blockifier_state_adapter;
mod call;
mod execution;
mod fee;
mod trace;

pub use block_context::ExecutionContext;
use blockifier::{
    state::cached_state::CommitmentStateDiff,
    transaction::{
        errors::TransactionExecutionError,
        objects::{FeeType, GasVector, TransactionExecutionInfo},
        transaction_types::TransactionType,
    },
};
use dc_db::{db_block_id::DbBlockId, storage_handler::DeoxysStorageError};
use starknet_api::transaction::TransactionHash;
use starknet_core::types::Felt;
pub use trace::execution_result_to_tx_trace;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Unsupported protocol version")]
    UnsupportedProtocolVersion,
    #[error("{0:#}")]
    Reexecution(#[from] TxReexecError),
    #[error("{0:#}")]
    FeeEstimation(#[from] TxFeeEstimationError),
    #[error("{0:#}")]
    MessageFeeEstimation(#[from] MessageFeeEstimationError),
    #[error("{0:#}")]
    CallContract(#[from] CallContractError),
    #[error("Storage error: {0:#}")]
    Storage(#[from] DeoxysStorageError),
}

#[derive(thiserror::Error, Debug)]
#[error("Reexecuting tx {hash:#} (index {index}) on top of {block_n}: {err:#}")]
pub struct TxReexecError {
    block_n: DbBlockId,
    hash: TransactionHash,
    index: usize,
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating fee for tx index {index} on top of {block_n}: {err:#}")]
pub struct TxFeeEstimationError {
    block_n: DbBlockId,
    index: usize,
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating message fee on top of {block_n}: {err:#}")]
pub struct MessageFeeEstimationError {
    block_n: DbBlockId,
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Calling contract {contract:#} on top of {block_n:#}: {err:#}")]
pub struct CallContractError {
    block_n: DbBlockId,
    contract: Felt,
    err: TransactionExecutionError,
}

pub struct ExecutionResult {
    pub hash: TransactionHash,
    pub tx_type: TransactionType,
    pub fee_type: FeeType,
    pub minimal_l1_gas: Option<GasVector>,
    pub execution_info: TransactionExecutionInfo,
    pub state_diff: CommitmentStateDiff,
}
