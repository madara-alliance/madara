//! Rust native contract execution for verification during tracing.
//!
//! This crate provides Rust implementations of Cairo contracts that can run
//! in parallel with Blockifier during transaction tracing to verify correctness.
//!
//! # Configuration
//!
//! Class hashes are configured via environment variables:
//!
//! ```bash
//! # Enable SimpleCounter verification
//! export RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH=0x0123456789abcdef...
//! ```
//!
//! See the [`config`] module for all available environment variables.

pub mod config;
pub mod context;
pub mod contracts;
pub mod gas;
pub mod state;
pub mod storage;
pub mod transaction;
pub mod types;
pub mod verify;

use starknet_types_core::felt::Felt;

pub use config::{
    account_class_hash, erc20_class_hash, is_verification_enabled, log_config_status, simple_counter_class_hash,
};
pub use context::ExecutionContext;
pub use contracts::{ContractRegistry, ExecutionError};
pub use gas::{BlockContext, FeeType, GasCosts, GasTracker, GasVector, ResourceBounds};
pub use state::{StateError, StateReader};
pub use transaction::{InvokeTransaction, TransactionExecutionResult, TransactionExecutor};
pub use types::{ContractAddress, ExecutionResult};
pub use verify::{VerificationError, VerificationResult};

/// Initialize the Rust execution verification system.
///
/// This should be called once at startup to log the configuration status.
/// It reads class hashes from environment variables and logs which contracts
/// are enabled for verification.
pub fn init() {
    log_config_status();
}

/// Main entry point for executing a transaction with Rust implementation.
///
/// This function:
/// 1. Checks if the contract/function is supported
/// 2. Executes the function using Rust implementation
/// 3. Returns the execution result
///
/// Returns `None` if the contract is not supported in Rust.
/// Returns `Some(Err(...))` if execution fails.
/// Returns `Some(Ok(...))` if execution succeeds.
pub fn execute_transaction<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Option<Result<ExecutionResult, ExecutionError>> {
    execute_transaction_with_timestamp(state, contract_address, class_hash, entry_point_selector, calldata, caller, 0)
}

/// Execute a transaction with explicit block timestamp.
///
/// Same as `execute_transaction` but with a block timestamp for contracts
/// that need to read/write timestamp-dependent storage.
pub fn execute_transaction_with_timestamp<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
    block_timestamp: u64,
) -> Option<Result<ExecutionResult, ExecutionError>> {
    // Check if this contract is supported
    if !ContractRegistry::supports_class_hash(class_hash) {
        return None;
    }

    // Check if this function is supported
    if !ContractRegistry::supports_function(class_hash, entry_point_selector) {
        return None;
    }

    // Execute the function
    ContractRegistry::execute_with_timestamp(
        state,
        contract_address,
        class_hash,
        entry_point_selector,
        calldata,
        caller,
        block_timestamp,
    )
}

/// Compare Rust execution result with Blockifier result.
///
/// This is a convenience function that delegates to the verify module.
pub fn compare_with_blockifier(
    rust_result: &ExecutionResult,
    blockifier_storage: &[(Felt, Vec<(Felt, Felt)>)],
    blockifier_retdata: &[Felt],
    blockifier_events: &[(Vec<Felt>, Vec<Felt>)],
    blockifier_failed: bool,
) -> VerificationResult {
    verify::verify_execution(rust_result, blockifier_storage, blockifier_retdata, blockifier_events, blockifier_failed)
}

/// Comprehensive comparison including all state changes.
///
/// This checks storage, retdata, events, nonces, L2-to-L1 messages, class hashes, and compiled class hashes.
pub fn compare_with_blockifier_comprehensive(
    rust_result: &ExecutionResult,
    blockifier_storage: &[(Felt, Vec<(Felt, Felt)>)],
    blockifier_retdata: &[Felt],
    blockifier_events: &[(Vec<Felt>, Vec<Felt>)],
    blockifier_failed: bool,
    blockifier_nonces: &[(Felt, Felt)],
    blockifier_messages: &[(Felt, Vec<Felt>)],
    blockifier_class_hashes: &[(Felt, Felt)],
    blockifier_compiled_class_hashes: &[(Felt, Felt)],
) -> VerificationResult {
    verify::verify_execution_comprehensive(
        rust_result,
        blockifier_storage,
        blockifier_retdata,
        blockifier_events,
        blockifier_failed,
        blockifier_nonces,
        blockifier_messages,
        blockifier_class_hashes,
        blockifier_compiled_class_hashes,
    )
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
    use crate::state::mock::MockStateReader;

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

    #[test]
    fn test_simple_counter_increment_with_env() {
        // This test verifies the contract execution logic works.
        // Since we can't easily reset Lazy statics, we test the execute function directly.
        use crate::contracts::simple_counter;

        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let selector = crate::storage::function_selector("increment");

        // Call execute directly (bypassing class hash check)
        let result = simple_counter::execute(&state, contract, selector, &[], ContractAddress(Felt::ZERO));

        assert!(result.is_ok());
        let execution_result = result.unwrap();
        // Cairo increment() has no return value
        assert!(execution_result.call_result.retdata.is_empty());
        assert!(!execution_result.call_result.failed);
        // Check storage was updated to 1 (0 + 1)
        assert!(execution_result.state_diff.storage_updates.contains_key(&contract));
    }

    #[test]
    fn test_simple_counter_get_counter_with_env() {
        use crate::contracts::simple_counter;
        use crate::contracts::simple_counter::layout::COUNTER_KEY;

        let mut state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        state.set_storage(contract, *COUNTER_KEY, Felt::from(42u64));

        let selector = crate::storage::function_selector("get_counter");

        // Call execute directly (bypassing class hash check)
        let result = simple_counter::execute(&state, contract, selector, &[], ContractAddress(Felt::ZERO));

        assert!(result.is_ok());
        let execution_result = result.unwrap();
        // get_counter() returns the current value
        assert_eq!(execution_result.call_result.retdata, vec![Felt::from(42u64)]);
        assert!(!execution_result.call_result.failed);
    }
}
