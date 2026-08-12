//! mc-rust-exec: native Rust transaction executor for the Madara hybrid pipeline.

pub mod config;
pub mod constants;
pub mod contracts;
pub mod core;
pub mod engine;
pub mod integration;
pub mod telemetry;
pub mod trace;

pub use core::{context, gas, state, storage, types};
pub use engine::{execution, runner, transaction};
pub use integration::{block_production, blockifier as blockifier_integration, compare, verification};
pub use telemetry::{hash_agg, storage_agg};

use starknet_types_core::felt::Felt;

pub const SUPPORTED_CONTRACTS: &str = include_str!(concat!(env!("OUT_DIR"), "/supported_contracts.json"));

pub use block_production::{execute_txns, RustDeferredReason, RustExecOutput, RustPhaseState};
pub use config::{
    account_class_hash, assets_manager_class_hash, erc20_class_hash, initialize_runtime_config,
    is_verification_enabled, log_config_status, paraclear_class_hash, paraclear_oracle_class_hash, runtime_config,
    RustExecRuntimeConfig,
};
pub use context::ExecutionContext;
pub use contracts::paradex::supported::{
    supported_class_hashes, supported_contracts, supported_selectors, SupportedContract, SupportedFunction,
};
pub use contracts::{ContractRegistry, ExecutionError};
pub use execution::{execute_transaction, execute_transaction_with_timestamp};
pub use gas::{BlockContext, FeeType, GasCosts, GasTracker, GasVector, ResourceBounds};
pub use runner::{execute_and_verify_with_timestamp, ExecutionVerificationResult};
pub use state::{StateError, StateReader};
pub use transaction::{InvokeTransaction, TransactionExecutionResult, TransactionExecutor};
pub use types::{ContractAddress, ExecutionResult};
pub use verification::{verify_against_blockifier, BlockifierExecutionResult};
pub use verification::{VerificationError, VerificationResult};

pub fn init() {
    log_config_status();
}

/// Get the human-readable name for a contract given its class hash.
///
/// Returns `None` if the class hash is not recognized.
pub fn get_contract_name(class_hash: Felt) -> Option<String> {
    ContractRegistry::get_contract_name(class_hash)
}

/// Get the human-readable name for a function given the class hash and selector.
///
/// Returns `None` if the function is not recognized.
pub fn get_function_name(class_hash: Felt, selector: Felt) -> Option<String> {
    ContractRegistry::get_function_name(class_hash, selector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::mock::MockStateReader;

    #[test]
    fn test_unsupported_contract_returns_none() {
        let state = MockStateReader::new();
        let unknown_class_hash = Felt::from(999u64);

        let result = execute_transaction(
            &state,
            ContractAddress(Felt::from(1u64)),
            unknown_class_hash,
            Felt::ZERO,
            &[],
            ContractAddress(Felt::ZERO),
        );

        assert!(result.is_none());
    }

    // Additional contract-specific tests removed along with non-Paradex contracts.
}
