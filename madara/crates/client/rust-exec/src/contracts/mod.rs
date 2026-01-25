//! Contract implementations.
//!
//! Each contract has its own submodule containing:
//! - Storage layout definitions
//! - Function implementations
//! - Contract metadata (class hash, supported selectors)

pub mod simple_counter;

use starknet_types_core::felt::Felt;

use crate::state::StateReader;
use crate::types::{ContractAddress, ExecutionResult};

/// Error returned when execution fails.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("State error: {0}")]
    State(#[from] crate::state::StateError),

    #[error("Unknown contract class hash: {0}")]
    UnknownClassHash(Felt),

    #[error("Unknown function selector: {0}")]
    UnknownSelector(Felt),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
}

/// Registry of known contracts and their implementations.
pub struct ContractRegistry;

impl ContractRegistry {
    /// Check if a class hash is supported by the Rust execution engine.
    pub fn supports_class_hash(class_hash: Felt) -> bool {
        simple_counter::CLASS_HASH == class_hash
    }

    /// Check if a (class_hash, selector) pair is supported.
    pub fn supports_function(class_hash: Felt, selector: Felt) -> bool {
        if class_hash == simple_counter::CLASS_HASH {
            return simple_counter::supports_selector(selector);
        }
        false
    }

    /// Execute a function on a contract.
    ///
    /// Returns `None` if the contract/function is not supported.
    /// Returns `Some(Err(...))` if execution fails.
    /// Returns `Some(Ok(...))` if execution succeeds.
    pub fn execute<S: StateReader>(
        state: &S,
        contract_address: ContractAddress,
        class_hash: Felt,
        selector: Felt,
        calldata: &[Felt],
        caller: ContractAddress,
    ) -> Option<Result<ExecutionResult, ExecutionError>> {
        if class_hash == simple_counter::CLASS_HASH {
            return Some(simple_counter::execute(state, contract_address, selector, calldata, caller));
        }

        None // Contract not supported
    }
}
