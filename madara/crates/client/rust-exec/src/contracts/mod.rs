//! Contract implementations.
//!
//! Each contract has its own submodule containing:
//! - Storage layout definitions
//! - Function implementations
//! - Contract metadata (class hash, supported selectors)
//!
//! # Configuration
//!
//! Class hashes are read from environment variables at startup.
//! See [`crate::config`] for details.

pub mod counter_with_event;
pub mod math_benchmark;
pub mod random_100_hashes;
pub mod simple_counter;

use starknet_types_core::felt::Felt;

use crate::config;
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
    ///
    /// This checks the class hash against configured environment variables.
    pub fn supports_class_hash(class_hash: Felt) -> bool {
        if let Some(simple_counter_hash) = config::simple_counter_class_hash() {
            if class_hash == simple_counter_hash {
                return true;
            }
        }
        if let Some(counter_with_event_hash) = config::counter_with_event_class_hash() {
            if class_hash == counter_with_event_hash {
                return true;
            }
        }
        if let Some(random_100_hashes_hash) = config::random_100_hashes_class_hash() {
            if class_hash == random_100_hashes_hash {
                return true;
            }
        }
        if let Some(math_benchmark_hash) = config::math_benchmark_class_hash() {
            if class_hash == math_benchmark_hash {
                return true;
            }
        }
        false
    }

    /// Check if a (class_hash, selector) pair is supported.
    pub fn supports_function(class_hash: Felt, selector: Felt) -> bool {
        if let Some(simple_counter_hash) = config::simple_counter_class_hash() {
            if class_hash == simple_counter_hash {
                return simple_counter::supports_selector(selector);
            }
        }
        if let Some(counter_with_event_hash) = config::counter_with_event_class_hash() {
            if class_hash == counter_with_event_hash {
                return counter_with_event::supports_selector(selector);
            }
        }
        if let Some(random_100_hashes_hash) = config::random_100_hashes_class_hash() {
            if class_hash == random_100_hashes_hash {
                return random_100_hashes::supports_selector(selector);
            }
        }
        if let Some(math_benchmark_hash) = config::math_benchmark_class_hash() {
            if class_hash == math_benchmark_hash {
                return math_benchmark::supports_selector(selector);
            }
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
        if let Some(simple_counter_hash) = config::simple_counter_class_hash() {
            if class_hash == simple_counter_hash {
                return Some(simple_counter::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(counter_with_event_hash) = config::counter_with_event_class_hash() {
            if class_hash == counter_with_event_hash {
                return Some(counter_with_event::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(random_100_hashes_hash) = config::random_100_hashes_class_hash() {
            if class_hash == random_100_hashes_hash {
                return Some(random_100_hashes::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(math_benchmark_hash) = config::math_benchmark_class_hash() {
            if class_hash == math_benchmark_hash {
                return Some(math_benchmark::execute(state, contract_address, selector, calldata, caller));
            }
        }

        None // Contract not supported
    }
}
