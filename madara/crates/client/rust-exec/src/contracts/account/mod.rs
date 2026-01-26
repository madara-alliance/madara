//! OpenZeppelin-style Account contract implementation.
//!
//! This module provides the account contract functionality needed for transaction execution:
//! - __validate__: Verify transaction signature
//! - __execute__: Execute multicall (dispatch calls to other contracts)
//!
//! # Configuration
//!
//! The class hash is configured via environment variable:
//! ```bash
//! export RUST_EXEC_ACCOUNT_CLASS_HASH=0x0123456789abcdef...
//! ```

pub mod functions;
pub mod layout;

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::function_selector;
use crate::types::{ContractAddress, ExecutionResult};

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "Account";

// Function selectors
fn validate_selector() -> Felt {
    function_selector("__validate__")
}

fn execute_selector() -> Felt {
    function_selector("__execute__")
}

fn is_valid_signature_selector() -> Felt {
    function_selector("is_valid_signature")
}

fn get_public_key_selector() -> Felt {
    function_selector("get_public_key")
}

fn set_public_key_selector() -> Felt {
    function_selector("set_public_key")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == validate_selector()
        || selector == execute_selector()
        || selector == is_valid_signature_selector()
        || selector == get_public_key_selector()
        || selector == set_public_key_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == validate_selector() {
        Some("__validate__".to_string())
    } else if selector == execute_selector() {
        Some("__execute__".to_string())
    } else if selector == is_valid_signature_selector() {
        Some("is_valid_signature".to_string())
    } else if selector == get_public_key_selector() {
        Some("get_public_key".to_string())
    } else if selector == set_public_key_selector() {
        Some("set_public_key".to_string())
    } else {
        None
    }
}

/// Execute a function on the Account contract.
pub fn execute<S: StateReader>(
    state: &S,
    account_address: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    _caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    if selector == validate_selector() {
        // __validate__(calls: Array<Call>)
        // In practice, signature comes from transaction, not calldata
        // For now, we just verify the account exists
        let _public_key = ctx.storage_read(state, account_address, *layout::ACCOUNT_PUBLIC_KEY)?;
        // Validation passes - return VALID
        ctx.set_retdata(vec![Felt::from_hex_unchecked("0x56414c4944")]);
    } else if selector == execute_selector() {
        // __execute__(calls: Array<Call>) -> Array<Span<felt252>>
        let _results = functions::execute_execute(state, account_address, calldata, &mut ctx)?;
    } else if selector == get_public_key_selector() {
        // get_public_key() -> felt252
        let public_key = ctx.storage_read(state, account_address, *layout::ACCOUNT_PUBLIC_KEY)?;
        ctx.set_retdata(vec![public_key]);
    } else if selector == set_public_key_selector() {
        // set_public_key(new_public_key: felt252)
        if calldata.len() != 1 {
            return Err(ExecutionError::ExecutionFailed("set_public_key takes 1 argument".to_string()));
        }
        ctx.storage_write(account_address, *layout::ACCOUNT_PUBLIC_KEY, calldata[0]);
        ctx.set_retdata(vec![]);
    } else if selector == is_valid_signature_selector() {
        // is_valid_signature(hash: felt252, signature: Array<felt252>) -> felt252
        // For simplicity, always return valid
        ctx.set_retdata(vec![Felt::from_hex_unchecked("0x56414c4944")]);
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

/// Execute __validate__ with explicit signature (for transaction validation).
pub fn validate_transaction<S: StateReader>(
    state: &S,
    account_address: ContractAddress,
    tx_hash: Felt,
    signature: &[Felt],
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();
    functions::execute_validate(state, account_address, tx_hash, signature, &mut ctx)?;
    Ok(ctx.build_result())
}

/// Execute __execute__ and return all call results (for transaction execution).
pub fn execute_transaction<S: StateReader>(
    state: &S,
    account_address: ContractAddress,
    calldata: &[Felt],
) -> Result<(ExecutionResult, Vec<Vec<Felt>>), ExecutionError> {
    let mut ctx = ExecutionContext::new();
    let results = functions::execute_execute(state, account_address, calldata, &mut ctx)?;
    Ok((ctx.build_result(), results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_validate() {
        assert!(supports_selector(validate_selector()));
    }

    #[test]
    fn test_supports_execute() {
        assert!(supports_selector(execute_selector()));
    }

    #[test]
    fn test_supports_get_public_key() {
        assert!(supports_selector(get_public_key_selector()));
    }
}
