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
mod names;

use starknet_types_core::felt::Felt;

use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::{event_selector, function_selector};
use crate::core::types::{ContractAddress, Event, ExecutionResult};

pub(crate) use names::PRECOMPUTED_NAMES;

const VALIDATE_INDEX: usize = 0;
const EXECUTE_INDEX: usize = 1;
const IS_VALID_SIGNATURE_INDEX: usize = 3;
const GET_PUBLIC_KEY_INDEX: usize = 4;
const SET_PUBLIC_KEY_INDEX: usize = 5;
const TRANSACTION_EXECUTED_EVENT_INDEX: usize = 6;

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "Account";

// Function selectors
fn validate_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[VALIDATE_INDEX])
}

fn execute_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[EXECUTE_INDEX])
}

fn is_valid_signature_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[IS_VALID_SIGNATURE_INDEX])
}

fn get_public_key_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[GET_PUBLIC_KEY_INDEX])
}

fn set_public_key_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[SET_PUBLIC_KEY_INDEX])
}

pub fn transaction_executed_event(account_class_hash: Felt, tx_hash: Felt, order: usize) -> Option<Event> {
    let argent_multiowner_class_hash =
        Felt::from_hex_unchecked("0x73414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2");
    (account_class_hash == argent_multiowner_class_hash).then(|| Event {
        order,
        keys: vec![event_selector(PRECOMPUTED_NAMES[TRANSACTION_EXECUTED_EVENT_INDEX]), tx_hash],
        data: Vec::new(),
    })
}

pub fn is_transaction_executed_event(event: &Event) -> bool {
    event.keys.first() == Some(&event_selector(PRECOMPUTED_NAMES[TRANSACTION_EXECUTED_EVENT_INDEX]))
        && event.keys.len() == 2
        && event.data.is_empty()
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
        Some(PRECOMPUTED_NAMES[VALIDATE_INDEX].to_string())
    } else if selector == execute_selector() {
        Some(PRECOMPUTED_NAMES[EXECUTE_INDEX].to_string())
    } else if selector == is_valid_signature_selector() {
        Some(PRECOMPUTED_NAMES[IS_VALID_SIGNATURE_INDEX].to_string())
    } else if selector == get_public_key_selector() {
        Some(PRECOMPUTED_NAMES[GET_PUBLIC_KEY_INDEX].to_string())
    } else if selector == set_public_key_selector() {
        Some(PRECOMPUTED_NAMES[SET_PUBLIC_KEY_INDEX].to_string())
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

    #[test]
    fn transaction_executed_event_matches_live_account_abi() {
        let tx_hash = Felt::from(0x123u64);
        let account_class_hash =
            Felt::from_hex_unchecked("0x73414441639dcd11d1846f287650a00c60c416b9d3ba45d31c651672125b2c2");
        let event = transaction_executed_event(account_class_hash, tx_hash, 7).expect("supported account event");

        assert_eq!(event.order, 7);
        assert_eq!(
            event.keys,
            vec![
                Felt::from_hex_unchecked("0x1dcde06aabdbca2f80aa51392b345d7549d7757aa855f7e37f5d335ac8243b1"),
                tx_hash
            ]
        );
        assert!(event.data.is_empty());
        assert!(is_transaction_executed_event(&event));
        assert!(transaction_executed_event(Felt::ZERO, tx_hash, 7).is_none());
    }
}
