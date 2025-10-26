#![allow(clippy::result_large_err)]

use blockifier::{
    state::cached_state::CommitmentStateDiff,
    transaction::{errors::TransactionExecutionError, objects::TransactionExecutionInfo},
};
use mp_chain_config::StarknetVersion;
use starknet_api::transaction::TransactionHash;
use starknet_api::{block::FeeType, executable_transaction::TransactionType};
use starknet_api::{execution_resources::GasVector, transaction::fields::GasVectorComputationMode};
use starknet_types_core::felt::Felt;

mod block_context;
mod blockifier_state_adapter;
mod call;
mod fee;
mod layered_state_adapter;
pub mod trace;

pub mod execution;
pub use block_context::*;
pub use blockifier_state_adapter::BlockifierStateAdapter;
pub use layered_state_adapter::LayeredStateAdapter;

/// Blockifier does not support execution for versions earlier than that.
pub const EXECUTION_UNSUPPORTED_BELOW_VERSION: StarknetVersion = StarknetVersion::V0_13_0;

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
#[error("Executing tx {hash:#} (index {index}) on top of {view}: {err:#}")]
pub struct TxExecError {
    view: String,
    hash: TransactionHash,
    index: usize,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating fee for tx index {index} on top of {view}: {err:#}")]
pub struct TxFeeEstimationError {
    view: String,
    index: usize,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating message fee on top of {view}: {err:#}")]
pub struct MessageFeeEstimationError {
    view: String,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Calling contract {contract:#x} on top of {view}: {err:#}")]
pub struct CallContractError {
    view: String,
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
    pub gas_vector_computation_mode: GasVectorComputationMode,
}
