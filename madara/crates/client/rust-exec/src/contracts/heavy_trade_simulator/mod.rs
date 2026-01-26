//! HeavyTradeSimulator contract implementation (idempotent version).
//!
//! This contract simulates the computational intensity of settle_trade_v3:
//! - ~770-790 Pedersen hash operations (storage reads/writes + computation phases)
//! - ~36 Poseidon hash operations (events)
//! - Complex mathematical operations (fees, PnL, funding)
//! - Multiple state changes across various storage mappings
//! - Comprehensive event emissions
//! - Fully deterministic (no timestamps, no block numbers)
//!
//! # Configuration
//!
//! The class hash is configured via environment variable:
//! ```bash
//! export RUST_EXEC_HEAVY_TRADE_SIMULATOR_CLASS_HASH=0x07f5d251afc92773f39cce9d6dabb6b80a5e72ab504ed0afbe840ffb1ba63939
//! ```

pub mod functions;
pub mod layout;
pub mod types;

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::function_selector;
use crate::types::{ContractAddress, ExecutionResult};

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "HeavyTradeSimulator";

// Function selectors
fn heavy_trade_simulation_selector() -> Felt {
    function_selector("heavy_trade_simulation")
}

fn get_account_state_selector() -> Felt {
    function_selector("get_account_state")
}

fn get_market_state_selector() -> Felt {
    function_selector("get_market_state")
}

fn get_total_hashes_computed_selector() -> Felt {
    function_selector("get_total_hashes_computed")
}

fn initialize_account_selector() -> Felt {
    function_selector("initialize_account")
}

fn initialize_market_selector() -> Felt {
    function_selector("initialize_market")
}

/// Check if this contract supports a given function selector.
pub fn supports_selector(selector: Felt) -> bool {
    selector == heavy_trade_simulation_selector()
        || selector == get_account_state_selector()
        || selector == get_market_state_selector()
        || selector == get_total_hashes_computed_selector()
        || selector == initialize_account_selector()
        || selector == initialize_market_selector()
}

/// Get the human-readable function name for a selector.
pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == heavy_trade_simulation_selector() {
        Some("heavy_trade_simulation".to_string())
    } else if selector == get_account_state_selector() {
        Some("get_account_state".to_string())
    } else if selector == get_market_state_selector() {
        Some("get_market_state".to_string())
    } else if selector == get_total_hashes_computed_selector() {
        Some("get_total_hashes_computed".to_string())
    } else if selector == initialize_account_selector() {
        Some("initialize_account".to_string())
    } else if selector == initialize_market_selector() {
        Some("initialize_market".to_string())
    } else {
        None
    }
}

/// Execute a function on the HeavyTradeSimulator contract.
pub fn execute<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    execute_with_timestamp(state, contract_address, selector, calldata, caller, 0)
}

/// Execute a function on the HeavyTradeSimulator contract with explicit block timestamp.
pub fn execute_with_timestamp<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    _caller: ContractAddress,
    block_timestamp: u64,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::with_timestamp(block_timestamp);

    if selector == heavy_trade_simulation_selector() {
        // heavy_trade_simulation(maker, taker, market_id, trade_size, trade_price, is_spot)
        if calldata.len() < 6 {
            return Err(ExecutionError::ExecutionFailed(
                "heavy_trade_simulation requires 6 arguments".to_string(),
            ));
        }
        let maker = ContractAddress(calldata[0]);
        let taker = ContractAddress(calldata[1]);
        let market_id = calldata[2];
        let trade_size = felt_to_u128(calldata[3])?;
        let trade_price = felt_to_u128(calldata[4])?;
        let is_spot = calldata[5] != Felt::ZERO;

        functions::execute_heavy_trade_simulation(
            state,
            contract_address,
            maker,
            taker,
            market_id,
            trade_size,
            trade_price,
            is_spot,
            &mut ctx,
        )?;
    } else if selector == get_account_state_selector() {
        // get_account_state(account: ContractAddress)
        if calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed(
                "get_account_state requires 1 argument".to_string(),
            ));
        }
        let account = ContractAddress(calldata[0]);
        functions::execute_get_account_state(state, contract_address, account, &mut ctx)?;
    } else if selector == get_market_state_selector() {
        // get_market_state(market_id: felt252)
        if calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed(
                "get_market_state requires 1 argument".to_string(),
            ));
        }
        let market_id = calldata[0];
        functions::execute_get_market_state(state, contract_address, market_id, &mut ctx)?;
    } else if selector == get_total_hashes_computed_selector() {
        // get_total_hashes_computed()
        functions::execute_get_total_hashes_computed(state, contract_address, &mut ctx)?;
    } else if selector == initialize_account_selector() {
        // initialize_account(account, market_id, initial_balance)
        if calldata.len() < 3 {
            return Err(ExecutionError::ExecutionFailed(
                "initialize_account requires 3 arguments".to_string(),
            ));
        }
        let account = ContractAddress(calldata[0]);
        let market_id = calldata[1];
        let initial_balance = felt_to_u128(calldata[2])?;
        functions::execute_initialize_account(contract_address, account, market_id, initial_balance, &mut ctx)?;
    } else if selector == initialize_market_selector() {
        // initialize_market(market_id)
        if calldata.is_empty() {
            return Err(ExecutionError::ExecutionFailed(
                "initialize_market requires 1 argument".to_string(),
            ));
        }
        let market_id = calldata[0];
        functions::execute_initialize_market(contract_address, market_id, &mut ctx)?;
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

/// Convert Felt to u128.
fn felt_to_u128(felt: Felt) -> Result<u128, ExecutionError> {
    let bytes = felt.to_bytes_be();
    if bytes.iter().take(16).any(|&b| b != 0) {
        return Err(ExecutionError::ExecutionFailed(
            "Value too large for u128".to_string(),
        ));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    Ok(u128::from_be_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_heavy_trade_simulation() {
        assert!(supports_selector(heavy_trade_simulation_selector()));
    }

    #[test]
    fn test_supports_get_account_state() {
        assert!(supports_selector(get_account_state_selector()));
    }

    #[test]
    fn test_get_function_name() {
        assert_eq!(
            get_function_name(heavy_trade_simulation_selector()),
            Some("heavy_trade_simulation".to_string())
        );
    }
}
