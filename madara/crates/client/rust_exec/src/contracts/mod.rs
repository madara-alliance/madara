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
pub mod devnet;
pub mod erc20;
pub mod paradex;

use starknet_types_core::felt::Felt;

use crate::config;
use crate::contracts::devnet::rust_exec_transfer;
use crate::contracts::paradex::{assets_manager, oracle, paraclear};
use crate::core::state::StateReader;
use crate::core::types::{ContractAddress, ExecutionResult};

/// Error returned when execution fails.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("State error: {0}")]
    State(#[from] crate::core::state::StateError),

    #[error("Unknown contract class hash: {0}")]
    UnknownClassHash(Felt),

    #[error("Unknown function selector: {0}")]
    UnknownSelector(Felt),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Invalid transaction nonce: expected {expected:#x}, got {actual:#x}")]
    InvalidNonce { expected: Felt, actual: Felt },
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
        if rust_exec_transfer::supports_class_hash(class_hash) {
            return Some(rust_exec_transfer::NAME.to_string());
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return Some(paraclear::NAME.to_string());
            }
        }
        if paraclear::supports_class_hash(class_hash) {
            return Some(paraclear::NAME.to_string());
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return Some("ParaclearOracle".to_string());
            }
        }
        if oracle::supports_class_hash(class_hash) {
            return Some("ParaclearOracle".to_string());
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
        if rust_exec_transfer::supports_class_hash(class_hash) {
            return rust_exec_transfer::get_function_name(selector);
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return paraclear::get_function_name(selector);
            }
        }
        if paraclear::supports_class_hash(class_hash) {
            return paraclear::get_function_name(selector);
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return oracle::get_function_name(selector);
            }
        }
        if oracle::supports_class_hash(class_hash) {
            return oracle::get_function_name(selector);
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
        if rust_exec_transfer::supports_class_hash(class_hash) {
            return true;
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return true;
            }
        }
        if paraclear::supports_class_hash(class_hash) {
            return true;
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return true;
            }
        }
        if oracle::supports_class_hash(class_hash) {
            return true;
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
        if rust_exec_transfer::supports_class_hash(class_hash) {
            return rust_exec_transfer::supports_selector(selector);
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
        if let Some(paraclear_hash) = config::paraclear_class_hash() {
            if class_hash == paraclear_hash {
                return paraclear::supports_selector(selector);
            }
        }
        if paraclear::supports_class_hash(class_hash) {
            return paraclear::supports_selector(selector);
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return oracle::supports_selector(selector);
            }
        }
        if oracle::supports_class_hash(class_hash) {
            return oracle::supports_selector(selector);
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
        if rust_exec_transfer::supports_class_hash(class_hash) {
            return Some(rust_exec_transfer::execute(state, contract_address, selector, calldata, caller));
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
        if paraclear::supports_class_hash(class_hash) {
            return Some(paraclear::execute_with_timestamp(
                state,
                contract_address,
                selector,
                calldata,
                caller,
                block_timestamp,
            ));
        }
        if let Some(oracle_hash) = config::paraclear_oracle_class_hash() {
            if class_hash == oracle_hash {
                return Some(oracle::execute(state, contract_address, selector, calldata, caller));
            }
        }
        if oracle::supports_class_hash(class_hash) {
            return Some(oracle::execute(state, contract_address, selector, calldata, caller));
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
        contracts::devnet::rust_exec_transfer,
        contracts::paradex::{oracle, paraclear},
        core::{state::mock::MockStateReader, types::ContractAddress},
        RustExecRuntimeConfig,
    };

    #[test]
    fn known_devnet_transfer_hash_dispatches_manifest_functions() {
        let transfer = crate::core::storage::function_selector("transfer");
        let mismatch = crate::core::storage::function_selector("transfer_with_comparator_mismatch");

        assert!(ContractRegistry::supports_class_hash(rust_exec_transfer::class_hash()));
        assert!(ContractRegistry::supports_function(rust_exec_transfer::class_hash(), transfer));
        assert!(ContractRegistry::supports_function(rust_exec_transfer::class_hash(), mismatch));
        assert_eq!(
            ContractRegistry::get_contract_name(rust_exec_transfer::class_hash()),
            Some("RustExecTransfer".to_string())
        );
    }

    #[test]
    fn generic_supported_class_hash_allows_oracle_selector_dispatch() {
        let oracle_class_hash = Felt::from_hex_unchecked("0x1234");
        config::initialize_runtime_config(RustExecRuntimeConfig {
            supported_contract_class_hashes: HashSet::from([oracle_class_hash]),
            ..Default::default()
        });

        let selector = crate::core::storage::function_selector("set_prices_and_funding_snapshot");
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

    #[test]
    fn known_paraclear_hash_dispatches_settle_trade_v3() {
        let selector = crate::core::storage::function_selector("settle_trade_v3");

        assert!(ContractRegistry::supports_class_hash(paraclear::CLASS_HASH));
        assert!(ContractRegistry::supports_function(paraclear::CLASS_HASH, selector));
        assert_eq!(ContractRegistry::get_contract_name(paraclear::CLASS_HASH), Some("Paraclear".to_string()));
        assert_eq!(
            ContractRegistry::get_function_name(paraclear::CLASS_HASH, selector),
            Some("settle_trade_v3".to_string())
        );

        let state = MockStateReader::new();
        let result = ContractRegistry::execute_with_timestamp(
            &state,
            ContractAddress(Felt::from_hex_unchecked("0x99")),
            paraclear::CLASS_HASH,
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

    #[test]
    fn known_oracle_hash_dispatches_set_prices_and_funding_snapshot() {
        let selector = crate::core::storage::function_selector("set_prices_and_funding_snapshot");

        assert!(ContractRegistry::supports_class_hash(oracle::CLASS_HASH));
        assert!(ContractRegistry::supports_function(oracle::CLASS_HASH, selector));
        assert_eq!(ContractRegistry::get_contract_name(oracle::CLASS_HASH), Some("ParaclearOracle".to_string()));
        assert_eq!(
            ContractRegistry::get_function_name(oracle::CLASS_HASH, selector),
            Some("set_prices_and_funding_snapshot".to_string())
        );
    }
}
