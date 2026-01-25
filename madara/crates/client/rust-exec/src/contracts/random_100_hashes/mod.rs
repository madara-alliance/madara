//! Random100Hashes contract implementation.
//!
//! This contract performs 100 Pedersen hash computations in a chain.
//! It's designed to benchmark compute-intensive operations.
//!
//! Functions:
//! - `compute_100_hashes(seed: felt252) -> felt252` - computes hash chain, returns result
//! - `get_last_result() -> felt252` - returns last computed hash
//!
//! # Configuration
//!
//! The class hash is configured via environment variable:
//! ```bash
//! export RUST_EXEC_RANDOM_100_HASHES_CLASS_HASH=0x0123456789abcdef...
//! ```
//!
//! See [`crate::config`] for details.

pub mod functions;
pub mod layout;

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::function_selector;
use crate::types::{ContractAddress, ExecutionResult};

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "Random100Hashes";

// Function selectors (computed as sn_keccak of function name)
fn compute_100_hashes_selector() -> Felt {
    function_selector("compute_100_hashes")
}

fn get_last_result_selector() -> Felt {
    function_selector("get_last_result")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == compute_100_hashes_selector() || selector == get_last_result_selector()
}

/// Execute a function on the Random100Hashes contract.
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

    if selector == compute_100_hashes_selector() {
        if calldata.len() != 1 {
            return Err(ExecutionError::ExecutionFailed(
                "compute_100_hashes() takes exactly 1 argument (seed)".to_string(),
            ));
        }
        let seed = calldata[0];
        functions::execute_compute_100_hashes(state, contract, seed, &mut ctx)?;
    } else if selector == get_last_result_selector() {
        if !calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed(
                "get_last_result() takes no arguments".to_string(),
            ));
        }
        functions::execute_get_last_result(state, contract, &mut ctx)?;
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
    fn test_supports_compute_100_hashes_selector() {
        assert!(supports_selector(compute_100_hashes_selector()));
    }

    #[test]
    fn test_supports_get_last_result_selector() {
        assert!(supports_selector(get_last_result_selector()));
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
    fn test_compute_100_hashes_missing_arg() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let result = execute(
            &state,
            contract,
            compute_100_hashes_selector(),
            &[],
            ContractAddress(Felt::ZERO),
        );
        assert!(matches!(result, Err(ExecutionError::ExecutionFailed(_))));
    }

    #[test]
    fn test_compute_100_hashes_success() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let seed = Felt::from(12345u64);

        let result = execute(
            &state,
            contract,
            compute_100_hashes_selector(),
            &[seed],
            ContractAddress(Felt::ZERO),
        )
        .unwrap();

        // Should return exactly 1 value
        assert_eq!(result.call_result.retdata.len(), 1);

        // Should have stored the result
        assert!(result.state_diff.storage_updates.contains_key(&contract));
    }
}
