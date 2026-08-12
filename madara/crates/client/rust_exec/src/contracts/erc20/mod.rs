//! ERC20 token contract implementation.
//!
//! This module provides ERC20 functionality needed for fee token transfers.
//! It supports both ETH and STRK fee tokens.

pub mod functions;
pub mod layout;
mod names;

use starknet_types_core::felt::Felt;

use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::function_selector;
use crate::core::types::{CallExecutionResult, ContractAddress, ExecutionResult, StateDiff};

pub(crate) use names::PRECOMPUTED_NAMES;

const BALANCE_OF_INDEX: usize = 1;
const TRANSFER_INDEX: usize = 2;
const TRANSFER_FROM_INDEX: usize = 3;
const TRANSFER_EVENT_INDEX: usize = 4;

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "ERC20";

// Function selectors
fn balance_of_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[BALANCE_OF_INDEX])
}

fn transfer_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[TRANSFER_INDEX])
}

fn transfer_from_selector() -> Felt {
    function_selector(PRECOMPUTED_NAMES[TRANSFER_FROM_INDEX])
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == balance_of_selector() || selector == transfer_selector() || selector == transfer_from_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == balance_of_selector() {
        Some(PRECOMPUTED_NAMES[BALANCE_OF_INDEX].to_string())
    } else if selector == transfer_selector() {
        Some(PRECOMPUTED_NAMES[TRANSFER_INDEX].to_string())
    } else if selector == transfer_from_selector() {
        Some(PRECOMPUTED_NAMES[TRANSFER_FROM_INDEX].to_string())
    } else {
        None
    }
}

/// Execute a function on the ERC20 contract.
pub fn execute<S: StateReader>(
    state: &S,
    token_address: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    if selector == balance_of_selector() {
        // balance_of(account: ContractAddress) -> u256
        if calldata.len() != 1 {
            return Err(ExecutionError::ExecutionFailed("balance_of takes 1 argument".to_string()));
        }
        let account = ContractAddress(calldata[0]);
        functions::execute_balance_of(state, token_address, account, &mut ctx)?;
    } else if selector == transfer_selector() {
        // transfer(recipient: ContractAddress, amount: u256) -> bool
        if calldata.len() != 3 {
            return Err(ExecutionError::ExecutionFailed(
                "transfer takes 3 arguments (to, amount_low, amount_high)".to_string(),
            ));
        }
        let to = ContractAddress(calldata[0]);
        let amount_low = felt_to_u128(calldata[1])?;
        let amount_high = felt_to_u128(calldata[2])?;

        // For transfer, caller is the sender (provided via syscall context)
        functions::execute_transfer(state, token_address, caller, to, amount_low, amount_high, &mut ctx)?;
    } else if selector == transfer_from_selector() {
        // transfer_from(sender: ContractAddress, recipient: ContractAddress, amount: u256) -> bool
        if calldata.len() != 4 {
            return Err(ExecutionError::ExecutionFailed(
                "transfer_from takes 4 arguments (from, to, amount_low, amount_high)".to_string(),
            ));
        }
        let from = ContractAddress(calldata[0]);
        let to = ContractAddress(calldata[1]);
        let amount_low = felt_to_u128(calldata[2])?;
        let amount_high = felt_to_u128(calldata[3])?;

        // For transfer_from, the first argument is the sender (from)
        // In a full implementation, we'd also check allowance from `from` to `caller`
        functions::execute_transfer(state, token_address, from, to, amount_low, amount_high, &mut ctx)?;
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

/// Internal transfer for fee payment (bypasses normal entry point).
///
/// This is called directly by the transaction executor to transfer fees.
pub fn transfer_internal<S: StateReader>(
    state: &S,
    token_address: ContractAddress,
    from: ContractAddress,
    to: ContractAddress,
    amount: u128,
    state_diff: &mut StateDiff,
) -> Result<CallExecutionResult, ExecutionError> {
    // Use the direct transfer that updates state_diff
    functions::transfer_internal_direct(state, token_address, from, to, amount, state_diff)?;

    // Return a successful call result
    Ok(CallExecutionResult {
        retdata: vec![Felt::ONE], // true
        events: vec![crate::core::types::Event {
            order: 0,
            keys: vec![crate::core::storage::sn_keccak(PRECOMPUTED_NAMES[TRANSFER_EVENT_INDEX].as_bytes())],
            data: vec![from.0, to.0, Felt::from(amount), Felt::ZERO],
        }],
        l2_to_l1_messages: vec![],
        failed: false,
        gas_consumed: 500, // Approximate gas for transfer
    })
}

/// Convert Felt to u128.
fn felt_to_u128(felt: Felt) -> Result<u128, ExecutionError> {
    let bytes = felt.to_bytes_be();
    if bytes.iter().take(16).any(|&b| b != 0) {
        return Err(ExecutionError::ExecutionFailed("Value too large for u128".to_string()));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    Ok(u128::from_be_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::mock::MockStateReader;

    #[test]
    fn test_supports_balance_of() {
        assert!(supports_selector(balance_of_selector()));
    }

    #[test]
    fn internal_fee_transfer_uses_legacy_event_abi() {
        let token = ContractAddress(Felt::from(1u64));
        let from = ContractAddress(Felt::from(2u64));
        let to = ContractAddress(Felt::from(3u64));
        let amount = 10u128;
        let mut state = MockStateReader::new();
        state.set_storage(token, layout::balance_key(from), Felt::from(100u64));
        let mut state_diff = StateDiff::default();

        let result = transfer_internal(&state, token, from, to, amount, &mut state_diff).expect("fee transfer");
        let event = result.events.first().expect("transfer event");

        assert_eq!(event.keys, vec![crate::core::storage::event_selector("Transfer")]);
        assert_eq!(event.data, vec![from.0, to.0, Felt::from(amount), Felt::ZERO]);
    }

    #[test]
    fn test_supports_transfer() {
        assert!(supports_selector(transfer_selector()));
    }
}
