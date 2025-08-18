use blockifier::{
    state::cached_state::CommitmentStateDiff,
    transaction::{errors::TransactionExecutionError, objects::TransactionExecutionInfo},
};
use mc_db::Anchor;
use starknet_api::execution_resources::GasVector;
use starknet_api::transaction::TransactionHash;
use starknet_api::{block::FeeType, executable_transaction::TransactionType};
use starknet_types_core::felt::Felt;

mod block_context;
mod blockifier_state_adapter;
mod call;
mod fee;
mod layered_state_adapter;
mod trace;

pub mod execution;
pub use block_context::*;
pub use blockifier_state_adapter::BlockifierStateAdapter;
pub use layered_state_adapter::LayeredStateAdapter;
pub use trace::execution_result_to_tx_trace;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    UnsupportedProtocolVersion(#[from] mp_chain_config::UnsupportedProtocolVersion),
    #[error(transparent)]
    Reexecution(#[from] TxExecError),
    #[error(transparent)]
    FeeEstimation(#[from] TxFeeEstimationError),
    #[error(transparent)]
    MessageFeeEstimation(#[from] MessageFeeEstimationError),
    #[error(transparent)]
    CallContract(#[from] CallContractError),
    #[error("Internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
    #[error("Invalid sequencer address: {0:#x}")]
    InvalidSequencerAddress(Felt),
}

#[derive(thiserror::Error, Debug)]
#[error("Executing tx {hash:#} (index {index}) on top of {anchor}: {err:#}")]
pub struct TxExecError {
    anchor: Anchor,
    hash: TransactionHash,
    index: usize,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating fee for tx index {index} on top of {anchor}: {err:#}")]
pub struct TxFeeEstimationError {
    anchor: Anchor,
    index: usize,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating message fee on top of {anchor}: {err:#}")]
pub struct MessageFeeEstimationError {
    anchor: Anchor,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Calling contract {contract:#x} on top of {anchor}: {err:#}")]
pub struct CallContractError {
    anchor: Anchor,
    contract: Felt,
    #[source]
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
