mod block_context;
mod blockifier_state_adapter;
mod execution;
mod fee;
mod trace;
pub use block_context::block_context;
use blockifier::{
    state::cached_state::CommitmentStateDiff,
    transaction::{
        errors::TransactionExecutionError,
        objects::{FeeType, GasVector, TransactionExecutionInfo},
        transaction_types::TransactionType,
    },
};
pub use execution::execute_transactions;
pub use fee::execution_result_to_fee_estimate;
use starknet_api::transaction::TransactionHash;
pub use trace::execution_result_to_tx_trace;

#[derive(thiserror::Error, Debug)]
#[error("Executing tx {hash:#} (index {index}): {err:#}")]
pub struct TransactionsExecError {
    pub hash: TransactionHash,
    pub index: usize,
    pub err: TransactionExecutionError,
}

pub struct ExecutionResult {
    pub hash: TransactionHash,
    pub tx_type: TransactionType,
    pub fee_type: FeeType,
    pub minimal_l1_gas: Option<GasVector>,
    pub execution_info: TransactionExecutionInfo,
    pub state_diff: CommitmentStateDiff,
}
