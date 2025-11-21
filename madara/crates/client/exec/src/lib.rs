//! Madara execution engine. This crate handles transaction execution, state management, and
//! block production for Madara nodes, providing the core execution capabilities that power
//! the Madara runtime.
//!
//! # Overview
//!
//! The execution module serves as the bridge between Madara's storage layer and the Starknet
//! execution environment (blockifier). It manages transaction execution, state transitions,
//! fee estimation, and call tracing. The module is designed to support multiple execution
//! contexts: block production, transaction validation, RPC calls, and state synchronization.
//!
//! The execution engine maintains consistency between the database state and the execution
//! environment through specialized state adapters that handle caching, layering, and
//! normalization of state changes.
//!
//! # Core Components
//!
//! The execution module is built around several key components that work together:
//!
//! - **Execution Context**: [`ExecutionContext`] manages the execution environment for a
//!   specific block, including block metadata and state visibility.
//! - **State Adapters**: [`BlockifierStateAdapter`] and [`LayeredStateAdapter`] provide
//!   different views of the blockchain state for various execution scenarios.
//! - **Backend Extensions**: [`MadaraBackendExecutionExt`] adds execution capabilities
//!   directly to the storage backend.
//! - **Transaction Processing**: Conversion utilities and execution flows for different
//!   transaction types
//!
//! # Execution Contexts
//!
//! The module supports multiple execution contexts, each optimized for different use cases:
//!
//! ## Block Production Context
//!
//! Used by sequencers to produce new blocks. This context uses [`LayeredStateAdapter`] to
//! maintain a cache of recent state changes that haven't been committed to the database yet.
//! This allows block production to continue even while previous blocks are being saved to db
//! asynchronously.
//!
//! ```no_run
//! let state_adapter = LayeredStateAdapter::new(backend.clone())?;
//! let executor = backend.new_executor_for_block_production(state_adapter, block_info)?;
//! ```
//!
//! ## Transaction Validation Context
//!
//! Used for validating incoming transactions against the current pending state. This context
//! operates on top of the pending block to ensure transactions are valid before adding them
//! to the mempool.
//!
//! ```no_run
//! let validator = backend.new_transaction_validator()?;
//! ```
//!
//! ## RPC Execution Context
//!
//! Used for RPC methods like `call`, `estimateFee`, and `simulateTransactions`. The context
//! can be initialized at different points in block execution:
//!
//! - **Block Start**: For tracing transaction execution without seeing state changes
//! - **Block End**: For estimating fees and simulating transactions with full state visibility
//!
//! ```no_run
//! let exec_context = ExecutionContext::new_at_block_start(backend.clone(), block_info)?;
//! let exec_context = ExecutionContext::new_at_block_end(backend.clone(), block_info)?;
//! ```
//!
//! # State Management
//!
//! The execution module uses several systems to handle different execution scenarios efficiently:
//!
//! ## BlockifierStateAdapter
//!
//! The [`BlockifierStateAdapter`] provides a read-only view of the blockchain state at a
//! specific block. It translates blockifier's state queries into database lookups, handling:
//!
//! - Contract storage retrieval
//! - Nonce management
//! - Class hash resolution
//! - Compiled class loading
//! - L1-to-L2 message consumption tracking
//!
//! ## LayeredStateAdapter
//!
//! The [`LayeredStateAdapter`] extends the basic state adapter with an in-memory cache layer
//! for block production scenarios. It maintains a queue of recent state changes organized by
//! block number, allowing execution to proceed even when recent blocks haven't been persisted
//! to the database yet.
//!
//! # Transaction Flows
//!
//! The execution of transactions follows the following steps
//!
//! ## Execution
//!
//! 1. **Context Initialization**: Create an [`ExecutionContext`] for the target block.
//! 2. **Transaction Conversion**: Convert API transaction to blockifier format.
//! 3. **State Preparation**: Initialize cached state with appropriate visibility.
//! 4. **Execution**: Run the transaction through blockifier.
//! 5. **Result Processing**: Extract execution info, state diff, and resource usage.
//!
//! ## Re-execution
//!
//! For transaction tracing and debugging, the module supports re-executing entire blocks:
//!
//! 1. **Baseline Execution**: Execute all transactions before the target range.
//! 2. **Traced Execution**: Execute target transactions with full tracing enabled.
//! 3. **Result Aggregation**: Collect execution results, traces, and state changes.
//!
//! This is primarily used by RPC methods like `traceTransaction` and `traceBlockTransactions`.
//!
//! ## Fee Estimation
//!
//! The module provides transaction fee estimation capabilities which accounts for:
//!
//! - **Gas Consumption**: L1, L2, and data availability gas usage
//! - **Gas Prices**: Current network gas price vectors
//! - **Tip Handling**: Transaction tip amounts for version 3 transactions
//! - **Minimal Gas**: Enforced minimum gas requirements
//! - **Fee Types**: Support for both ETH and STRK fee tokens
//!
//! The entry point for fee estimation is the [`execution_result_to_fee_estimate`] method.
//!
//! # State Diff Management
//!
//! State changes during execution are kept track of and normalized. The
//! [`create_normalized_state_diff`] function processes raw state changes from blockifier and
//! creates a normalized state diff by:
//!
//! 1. **Change Detection**: Comparing new values against database state
//! 2. **Deduplication**: Removing changes that result in the original value
//! 3. **Classification**: Distinguishing between deployments, replacements, and updates
//! 4. **Sorting**: Organizing changes for consistent output
//!
//! The module handles various types of state changes:
//!
//! - **Storage Updates**: Contract storage key-value changes
//! - **Nonce Updates**: Account nonce increments
//! - **Class Declarations**: New class deployments (Sierra and legacy)
//! - **Contract Deployments**: New contract address assignments
//! - **Class Replacements**: Contract class upgrades
//!
//! [`ExecutionContext`]: crate::ExecutionContext
//! [`BlockifierStateAdapter`]: crate::BlockifierStateAdapter
//! [`LayeredStateAdapter`]: crate::LayeredStateAdapter
//! [`MadaraBackendExecutionExt`]: crate::MadaraBackendExecutionExt
//! [`execution_result_to_fee_estimate`]: crate::ExecutionContext::execution_result_to_fee_estimate
//! [`create_normalized_state_diff`]: crate::state_diff::create_normalized_state_diff
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
///
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
