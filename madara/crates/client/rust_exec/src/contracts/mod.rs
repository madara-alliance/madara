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
pub mod erc20;
pub mod paradex;
pub mod paradex_codegen;

use starknet_types_core::felt::Felt;

use crate::config;
use crate::contracts::paradex::{assets_manager, oracle, paraclear};
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

#[derive(Clone, Copy)]
enum ParadexContractFamily {
    Paraclear,
    Oracle,
    AssetsManager,
}

fn infer_paradex_contract_family(selector: Felt) -> Option<ParadexContractFamily> {
    if paraclear::supports_selector(selector) {
        Some(ParadexContractFamily::Paraclear)
    } else if oracle::supports_selector(selector) {
        Some(ParadexContractFamily::Oracle)
    } else if assets_manager::supports_selector(selector) {
        Some(ParadexContractFamily::AssetsManager)
    } else {
        None
    }
}

impl ContractRegistry {
    /// Get the human-readable name for a contract given its class hash.
    ///
    /// Returns `None` if the class hash is not recognized.
    pub fn get_contract_name(class_hash: Felt) -> Option<String> {
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return Some(paraclear::NAME.to_string());
            }
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return Some("ParaclearOracle".to_string());
            }
        }
        if let Some(assets_manager_hash) = config::assets_manager_class_hash() {
            if class_hash == assets_manager_hash {
                return Some("AssetsManager".to_string());
            }
        }
        None
    }

    /// Get the human-readable name for a function given the class hash and selector.
    ///
    /// Returns `None` if the function is not recognized.
    pub fn get_function_name(class_hash: Felt, selector: Felt) -> Option<String> {
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return paraclear::get_function_name(selector);
            }
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return oracle::get_function_name(selector);
            }
        }
        if let Some(assets_manager_hash) = config::assets_manager_class_hash() {
            if class_hash == assets_manager_hash {
                return assets_manager::get_function_name(selector);
            }
        }
        if config::supports_runtime_class_hash(class_hash) {
            return match infer_paradex_contract_family(selector) {
                Some(ParadexContractFamily::Paraclear) => paraclear::get_function_name(selector),
                Some(ParadexContractFamily::Oracle) => oracle::get_function_name(selector),
                Some(ParadexContractFamily::AssetsManager) => assets_manager::get_function_name(selector),
                None => None,
            };
        }
        None
    }

    /// Check if a class hash is supported by the Rust execution engine.
    ///
    /// This checks the class hash against configured environment variables.
    pub fn supports_class_hash(class_hash: Felt) -> bool {
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return true;
            }
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return true;
            }
        }
        if let Some(assets_manager_hash) = config::assets_manager_class_hash() {
            if class_hash == assets_manager_hash {
                return true;
            }
        }
        config::supports_runtime_class_hash(class_hash)
    }

    /// Check if a (class_hash, selector) pair is supported.
    pub fn supports_function(class_hash: Felt, selector: Felt) -> bool {
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return paraclear::supports_selector(selector);
            }
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return oracle::supports_selector(selector);
            }
        }
        if let Some(assets_manager_hash) = config::assets_manager_class_hash() {
            if class_hash == assets_manager_hash {
                return assets_manager::supports_selector(selector);
            }
        }
        if config::supports_runtime_class_hash(class_hash) {
            return infer_paradex_contract_family(selector).is_some();
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return Some(paraclear::execute_with_timestamp(
                    state,
                    contract_address,
                    selector,
                    calldata,
                    caller,
                    block_timestamp,
                ));
            }
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return Some(oracle::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if let Some(assets_manager_hash) = config::assets_manager_class_hash() {
            if class_hash == assets_manager_hash {
                return Some(assets_manager::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if config::supports_runtime_class_hash(class_hash) {
            return match infer_paradex_contract_family(selector) {
                Some(ParadexContractFamily::Paraclear) => Some(paraclear::execute_with_timestamp(
                    state,
                    contract_address,
                    selector,
                    calldata,
                    caller,
                    block_timestamp,
                )),
                Some(ParadexContractFamily::Oracle) => {
                    Some(oracle::execute(state, contract_address, selector, calldata, caller))
                }
                Some(ParadexContractFamily::AssetsManager) => {
                    Some(assets_manager::execute(state, contract_address, selector, calldata, caller))
                }
                None => None,
            };
        }

        None // Contract not supported
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use starknet_types_core::felt::Felt;

    use super::{config, ContractRegistry, ExecutionError};
    use crate::{
        contracts::paradex::oracle, state::mock::MockStateReader, types::ContractAddress, RustExecRuntimeConfig,
    };

    #[test]
    fn generic_supported_class_hash_allows_oracle_selector_dispatch() {
        let oracle_class_hash = Felt::from_hex_unchecked("0x1234");
        config::initialize_runtime_config(RustExecRuntimeConfig {
            supported_contract_class_hashes: HashSet::from([oracle_class_hash]),
            ..Default::default()
        });

        let selector = crate::storage::function_selector("set_prices_and_funding_snapshot");
        assert!(oracle::supports_selector(selector));
        assert!(ContractRegistry::supports_class_hash(oracle_class_hash));
        assert!(ContractRegistry::supports_function(oracle_class_hash, selector));
        assert_eq!(
            ContractRegistry::get_function_name(oracle_class_hash, selector),
            Some("set_prices_and_funding_snapshot".to_string())
        );

        let state = MockStateReader::new();
        let result = ContractRegistry::execute_with_timestamp(
            &state,
            ContractAddress(Felt::from_hex_unchecked("0x99")),
            oracle_class_hash,
            selector,
            &[],
            ContractAddress(Felt::ZERO),
            0,
        );

        assert!(matches!(
            result,
            Some(Err(ExecutionError::ExecutionFailed(message))) if message.contains("calldata underflow")
        ));
    }
}
