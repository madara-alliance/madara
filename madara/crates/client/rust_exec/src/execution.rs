//! Rust transaction execution entry points.

use starknet_types_core::felt::Felt;

use crate::{contracts::ContractRegistry, types::ContractAddress, ExecutionError, ExecutionResult, StateReader};

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
