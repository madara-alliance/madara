//! SimpleCounter contract implementation.
//!
//! This contract has a single storage variable `counter: u128` and functions:
//! - `get_counter() -> u128` - returns current value
//! - `increment()` - adds 1 to counter
//! - `decrement()` - subtracts 1 from counter
//! - `set_counter(value: u128)` - sets counter to value
//!
//! # Configuration
//!
//! The class hash is configured via environment variable:
//! ```bash
//! export RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH=0x0123456789abcdef...
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
pub const NAME: &str = "SimpleCounter";

// Function selectors (computed as sn_keccak of function name)
fn get_counter_selector() -> Felt {
    function_selector("get_counter")
}

fn increment_selector() -> Felt {
    function_selector("increment")
}

fn decrement_selector() -> Felt {
    function_selector("decrement")
}

fn set_counter_selector() -> Felt {
    function_selector("set_counter")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == get_counter_selector()
        || selector == increment_selector()
        || selector == decrement_selector()
        || selector == set_counter_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == get_counter_selector() {
        Some("get_counter".to_string())
    } else if selector == increment_selector() {
        Some("increment".to_string())
    } else if selector == decrement_selector() {
        Some("decrement".to_string())
    } else if selector == set_counter_selector() {
        Some("set_counter".to_string())
    } else {
        None
    }
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

    if selector == get_counter_selector() {
        if !calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed("get_counter() takes no arguments".to_string()));
        }
        functions::execute_get_counter(state, contract, &mut ctx)?;
    } else if selector == increment_selector() {
        if !calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed("increment() takes no arguments".to_string()));
        }
        functions::execute_increment(state, contract, &mut ctx)?;
    } else if selector == decrement_selector() {
        if !calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed("decrement() takes no arguments".to_string()));
        }
        functions::execute_decrement(state, contract, &mut ctx)?;
    } else if selector == set_counter_selector() {
        if calldata.len() != 1 {
            return Err(ExecutionError::ExecutionFailed("set_counter() takes exactly 1 argument".to_string()));
        }
        functions::execute_set_counter(state, contract, &mut ctx, calldata[0])?;
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
