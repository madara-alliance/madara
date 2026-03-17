//! ERC20 token contract implementation.
//!
//! This module provides ERC20 functionality needed for fee token transfers.
//! It supports both ETH and STRK fee tokens.

pub mod functions;
pub mod layout;

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::function_selector;
use crate::types::{CallExecutionResult, ContractAddress, ExecutionResult, StateDiff};

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "ERC20";

// Function selectors
fn balance_of_selector() -> Felt {
    function_selector("balance_of")
}

fn transfer_selector() -> Felt {
    function_selector("transfer")
}

fn transfer_from_selector() -> Felt {
    function_selector("transfer_from")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == balance_of_selector() || selector == transfer_selector() || selector == transfer_from_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == balance_of_selector() {
        Some("balance_of".to_string())
    } else if selector == transfer_selector() {
        Some("transfer".to_string())
    } else if selector == transfer_from_selector() {
        Some("transfer_from".to_string())
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
        events: vec![
            // Transfer event
            crate::types::Event {
                order: 0,
                keys: vec![crate::storage::sn_keccak(b"Transfer"), from.0, to.0],
                data: vec![Felt::from(amount), Felt::ZERO], // amount as u256
            },
        ],
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

    #[test]
    fn test_supports_balance_of() {
        assert!(supports_selector(balance_of_selector()));
    }

    #[test]
    fn test_supports_transfer() {
        assert!(supports_selector(transfer_selector()));
    }
}
