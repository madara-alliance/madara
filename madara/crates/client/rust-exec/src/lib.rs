//! Rust native contract execution for verification during tracing.
//!
//! This crate provides Rust implementations of Cairo contracts that can run
//! in parallel with Blockifier during transaction tracing to verify correctness.

pub mod context;
pub mod contracts;
pub mod state;
pub mod storage;
pub mod types;
pub mod verify;

use starknet_types_core::felt::Felt;

pub use context::ExecutionContext;
pub use contracts::{ContractRegistry, ExecutionError};
pub use state::{StateError, StateReader};
pub use types::{ContractAddress, ExecutionResult};
pub use verify::{VerificationError, VerificationResult};

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
    // Check if this contract is supported
    if !ContractRegistry::supports_class_hash(class_hash) {
        return None;
    }

    // Check if this function is supported
    if !ContractRegistry::supports_function(class_hash, entry_point_selector) {
        return None;
    }

    // Execute the function
    ContractRegistry::execute(state, contract_address, class_hash, entry_point_selector, calldata, caller)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::simple_counter;
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
    fn test_simple_counter_execution() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let class_hash = simple_counter::CLASS_HASH;
        let selector = crate::storage::function_selector("increment");

        let result = execute_transaction(&state, contract, class_hash, selector, &[], ContractAddress(Felt::ZERO));

        assert!(result.is_some());
        let execution_result = result.unwrap().unwrap();
        assert_eq!(execution_result.call_result.retdata, vec![Felt::TWO]);
        assert!(!execution_result.call_result.failed);
    }
}
