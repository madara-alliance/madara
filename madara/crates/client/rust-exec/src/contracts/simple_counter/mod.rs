//! SimpleCounter contract implementation.
//!
//! This is a simple contract with a single storage variable `x: felt252`
//! and a function `increment()` that adds 2 to `x`.

pub mod functions;
pub mod layout;

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::function_selector;
use crate::types::{ContractAddress, ExecutionResult};

/// Class hash for SimpleCounter contract.
///
/// This should match the class hash of the deployed contract.
/// For now, using a placeholder value. In production, this would be
/// the actual class hash from the Cairo compiler.
pub const CLASS_HASH: Felt = Felt::from_hex_unchecked("0x1234567890abcdef");

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "SimpleCounter";

/// Get the function selector for increment().
///
/// In production, this would be computed as sn_keccak("increment").
fn increment_selector() -> Felt {
    function_selector("increment")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == increment_selector()
}

/// Execute a function on the SimpleCounter contract.
///
/// Returns the execution result or an error.
pub fn execute<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    _caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    if selector == increment_selector() {
        // increment() takes no arguments
        if !calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed("increment() takes no arguments".to_string()));
        }

        functions::execute_increment(state, contract, &mut ctx)?;
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_supports_increment_selector() {
        assert!(supports_selector(increment_selector()));
    }

    #[test]
    fn test_unknown_selector() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let unknown_selector = Felt::from(999u64);

        let result = execute(&state, contract, unknown_selector, &[], ContractAddress(Felt::ZERO));
        assert!(matches!(result, Err(ExecutionError::UnknownSelector(_))));
    }

    #[test]
    fn test_increment_with_calldata_fails() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let result = execute(&state, contract, increment_selector(), &[Felt::from(1u64)], ContractAddress(Felt::ZERO));
        assert!(matches!(result, Err(ExecutionError::ExecutionFailed(_))));
    }
}
