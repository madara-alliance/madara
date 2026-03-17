//! Orchestrates Rust execution and verification against Blockifier.

use std::time::Duration;

use starknet_types_core::felt::Felt;

use crate::{
    execution::execute_transaction_with_timestamp,
    verification::{verify_against_blockifier, BlockifierExecutionResult, VerificationResult},
    ContractAddress, ExecutionResult, StateReader,
};

/// Outcome of running Rust execution and verifying against Blockifier.
#[derive(Debug)]
pub struct ExecutionVerificationResult {
    pub executed: bool,
    pub passed: bool,
    pub execution_duration: Duration,
    pub rust_result: Option<ExecutionResult>,
    pub verification: Option<VerificationResult>,
}

/// Execute the transaction in Rust and verify against a Blockifier result.
pub fn execute_and_verify_with_timestamp<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    class_hash: Felt,
    entry_point_selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
    block_timestamp: u64,
    blockifier_result: &BlockifierExecutionResult,
) -> ExecutionVerificationResult {
    let rust_result = execute_transaction_with_timestamp(
        state,
        contract_address,
        class_hash,
        entry_point_selector,
        calldata,
        caller,
        block_timestamp,
    );

    let rust_exec_duration = Duration::ZERO;

    match rust_result {
        Some(Ok(result)) => {
            let verification = verify_against_blockifier(&result, blockifier_result);
            ExecutionVerificationResult {
                executed: true,
                passed: verification.passed,
                execution_duration: rust_exec_duration,
                rust_result: Some(result),
                verification: Some(verification),
            }
        }
        Some(Err(_)) => ExecutionVerificationResult {
            executed: true,
            passed: false,
            execution_duration: rust_exec_duration,
            rust_result: None,
            verification: None,
        },
        None => ExecutionVerificationResult {
            executed: false,
            passed: false,
            execution_duration: Duration::ZERO,
            rust_result: None,
            verification: None,
        },
    }
}
