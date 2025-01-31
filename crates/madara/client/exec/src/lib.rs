use core::fmt;

use blockifier::{
    state::cached_state::CommitmentStateDiff,
    transaction::{
        errors::TransactionExecutionError, objects::TransactionExecutionInfo, transaction_types::TransactionType,
    },
};
use mc_db::{db_block_id::DbBlockId, MadaraStorageError};
use starknet_api::block::FeeType;
use starknet_api::execution_resources::GasVector;
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;

mod block_context;
mod blockifier_state_adapter;
mod call;
pub mod execution;
mod fee;
mod trace;
pub mod transaction;

pub use block_context::ExecutionContext;
pub use blockifier_state_adapter::BlockifierStateAdapter;
pub use trace::execution_result_to_tx_trace;

#[derive(Debug)]
struct OnTopOf(Option<DbBlockId>);
impl fmt::Display for OnTopOf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(id) => write!(f, "{:#}", id),
            None => write!(f, "<none>"),
        }
    }
}
impl From<Option<DbBlockId>> for OnTopOf {
    fn from(value: Option<DbBlockId>) -> Self {
        Self(value)
    }
}

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
    #[error("Storage error: {0:#}")]
    Storage(#[from] MadaraStorageError),
    #[error("Invalid sequencer address: {0:#x}")]
    InvalidSequencerAddress(Felt),
}

#[derive(thiserror::Error, Debug)]
#[error("Executing tx {hash:#} (index {index}) on top of {block_n}: {err:#}")]
pub struct TxExecError {
    block_n: OnTopOf,
    hash: TransactionHash,
    index: usize,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating fee for tx index {index} on top of {block_n}: {err:#}")]
pub struct TxFeeEstimationError {
    block_n: OnTopOf,
    index: usize,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Estimating message fee on top of {block_n}: {err:#}")]
pub struct MessageFeeEstimationError {
    block_n: OnTopOf,
    #[source]
    err: TransactionExecutionError,
}

#[derive(thiserror::Error, Debug)]
#[error("Calling contract {contract:#x} on top of {block_n}: {err:#}")]
pub struct CallContractError {
    block_n: OnTopOf,
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
