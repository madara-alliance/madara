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

pub mod account;
pub mod comprehensive_benchmark;
pub mod counter_with_event;
pub mod erc20;
pub mod heavy_trade_simulator;
pub mod math_benchmark;
pub mod random_100_hashes;
pub mod simple_counter;
pub mod storage_heavy;

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
    /// Get the human-readable name for a contract given its class hash.
    ///
    /// Returns `None` if the class hash is not recognized.
    pub fn get_contract_name(class_hash: Felt) -> Option<String> {
        if let Some(simple_counter_hash) = config::simple_counter_class_hash() {
            if class_hash == simple_counter_hash {
                return Some(simple_counter::NAME.to_string());
            }
        }
        if let Some(counter_with_event_hash) = config::counter_with_event_class_hash() {
            if class_hash == counter_with_event_hash {
                return Some(counter_with_event::NAME.to_string());
            }
        }
        if let Some(random_100_hashes_hash) = config::random_100_hashes_class_hash() {
            if class_hash == random_100_hashes_hash {
                return Some(random_100_hashes::NAME.to_string());
            }
        }
        if let Some(math_benchmark_hash) = config::math_benchmark_class_hash() {
            if class_hash == math_benchmark_hash {
                return Some(math_benchmark::NAME.to_string());
            }
        }
        if let Some(account_hash) = config::account_class_hash() {
            if class_hash == account_hash {
                return Some(account::NAME.to_string());
            }
        }
        if let Some(erc20_hash) = config::erc20_class_hash() {
            if class_hash == erc20_hash {
                return Some(erc20::NAME.to_string());
            }
        }
        if let Some(heavy_trade_hash) = config::heavy_trade_simulator_class_hash() {
            if class_hash == heavy_trade_hash {
                return Some(heavy_trade_simulator::NAME.to_string());
            }
        }
        if let Some(storage_heavy_hash) = config::storage_heavy_class_hash() {
            if class_hash == storage_heavy_hash {
                return Some(storage_heavy::NAME.to_string());
            }
        }
        if let Some(comprehensive_benchmark_hash) = config::comprehensive_benchmark_class_hash() {
            if class_hash == comprehensive_benchmark_hash {
                return Some(comprehensive_benchmark::NAME.to_string());
            }
        }
        None
    }

    /// Get the human-readable name for a function given the class hash and selector.
    ///
    /// Returns `None` if the function is not recognized.
    pub fn get_function_name(class_hash: Felt, selector: Felt) -> Option<String> {
        if let Some(simple_counter_hash) = config::simple_counter_class_hash() {
            if class_hash == simple_counter_hash {
                return simple_counter::get_function_name(selector);
            }
        }
        if let Some(counter_with_event_hash) = config::counter_with_event_class_hash() {
            if class_hash == counter_with_event_hash {
                return counter_with_event::get_function_name(selector);
            }
        }
        if let Some(random_100_hashes_hash) = config::random_100_hashes_class_hash() {
            if class_hash == random_100_hashes_hash {
                return random_100_hashes::get_function_name(selector);
            }
        }
        if let Some(math_benchmark_hash) = config::math_benchmark_class_hash() {
            if class_hash == math_benchmark_hash {
                return math_benchmark::get_function_name(selector);
            }
        }
        if let Some(account_hash) = config::account_class_hash() {
            if class_hash == account_hash {
                return account::get_function_name(selector);
            }
        }
        if let Some(erc20_hash) = config::erc20_class_hash() {
            if class_hash == erc20_hash {
                return erc20::get_function_name(selector);
            }
        }
        if let Some(heavy_trade_hash) = config::heavy_trade_simulator_class_hash() {
            if class_hash == heavy_trade_hash {
                return heavy_trade_simulator::get_function_name(selector);
            }
        }
        if let Some(storage_heavy_hash) = config::storage_heavy_class_hash() {
            if class_hash == storage_heavy_hash {
                return storage_heavy::get_function_name(selector);
            }
        }
        if let Some(comprehensive_benchmark_hash) = config::comprehensive_benchmark_class_hash() {
            if class_hash == comprehensive_benchmark_hash {
                return comprehensive_benchmark::get_function_name(selector);
            }
        }
        None
    }

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
        if let Some(account_hash) = config::account_class_hash() {
            if class_hash == account_hash {
                return true;
            }
        }
        if let Some(erc20_hash) = config::erc20_class_hash() {
            if class_hash == erc20_hash {
                return true;
            }
        }
        if let Some(heavy_trade_hash) = config::heavy_trade_simulator_class_hash() {
            if class_hash == heavy_trade_hash {
                return true;
            }
        }
        if let Some(storage_heavy_hash) = config::storage_heavy_class_hash() {
            if class_hash == storage_heavy_hash {
                return true;
            }
        }
        if let Some(comprehensive_benchmark_hash) = config::comprehensive_benchmark_class_hash() {
            if class_hash == comprehensive_benchmark_hash {
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
        if let Some(account_hash) = config::account_class_hash() {
            if class_hash == account_hash {
                return account::supports_selector(selector);
            }
        }
        if let Some(erc20_hash) = config::erc20_class_hash() {
            if class_hash == erc20_hash {
                return erc20::supports_selector(selector);
            }
        }
        if let Some(heavy_trade_hash) = config::heavy_trade_simulator_class_hash() {
            if class_hash == heavy_trade_hash {
                return heavy_trade_simulator::supports_selector(selector);
            }
        }
        if let Some(storage_heavy_hash) = config::storage_heavy_class_hash() {
            if class_hash == storage_heavy_hash {
                return storage_heavy::supports_selector(selector);
            }
        }
        if let Some(comprehensive_benchmark_hash) = config::comprehensive_benchmark_class_hash() {
            if class_hash == comprehensive_benchmark_hash {
                return comprehensive_benchmark::supports_selector(selector);
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
        Self::execute_with_timestamp(state, contract_address, class_hash, selector, calldata, caller, 0)
    }

    /// Execute a function on a contract with explicit block timestamp.
    pub fn execute_with_timestamp<S: StateReader>(
        state: &S,
        contract_address: ContractAddress,
        class_hash: Felt,
        selector: Felt,
        calldata: &[Felt],
        caller: ContractAddress,
        block_timestamp: u64,
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
        if let Some(account_hash) = config::account_class_hash() {
            if class_hash == account_hash {
                return Some(account::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(erc20_hash) = config::erc20_class_hash() {
            if class_hash == erc20_hash {
                return Some(erc20::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(heavy_trade_hash) = config::heavy_trade_simulator_class_hash() {
            if class_hash == heavy_trade_hash {
                return Some(heavy_trade_simulator::execute_with_timestamp(
                    state,
                    contract_address,
                    selector,
                    calldata,
                    caller,
                    block_timestamp,
                ));
            }
        }
        if let Some(storage_heavy_hash) = config::storage_heavy_class_hash() {
            if class_hash == storage_heavy_hash {
                return Some(storage_heavy::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(comprehensive_benchmark_hash) = config::comprehensive_benchmark_class_hash() {
            if class_hash == comprehensive_benchmark_hash {
                return Some(comprehensive_benchmark::execute(state, contract_address, selector, calldata, caller));
            }
        }

        None // Contract not supported
    }
}
