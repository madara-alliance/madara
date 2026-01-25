//! MathBenchmark contract implementation.
//!
//! This contract benchmarks various mathematical operations to compare
//! Rust (native i128/u128) vs Cairo (felt252) performance.
//!
//! Based on settle_trade_v3 patterns with realistic trading values.

pub mod functions;
pub mod layout;

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::function_selector;
use crate::types::{ContractAddress, ExecutionResult};

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "MathBenchmark";

// Function selectors (computed as sn_keccak of function name)
fn benchmark_division_selector() -> Felt {
    function_selector("benchmark_division")
}

fn benchmark_multiplication_selector() -> Felt {
    function_selector("benchmark_multiplication")
}

fn benchmark_addition_selector() -> Felt {
    function_selector("benchmark_addition")
}

fn benchmark_subtraction_selector() -> Felt {
    function_selector("benchmark_subtraction")
}

fn benchmark_fee_calculation_selector() -> Felt {
    function_selector("benchmark_fee_calculation")
}

fn benchmark_signed_division_selector() -> Felt {
    function_selector("benchmark_signed_division")
}

fn benchmark_signed_multiplication_selector() -> Felt {
    function_selector("benchmark_signed_multiplication")
}

fn benchmark_events_selector() -> Felt {
    function_selector("benchmark_events")
}

fn benchmark_assertions_selector() -> Felt {
    function_selector("benchmark_assertions")
}

fn benchmark_full_trade_selector() -> Felt {
    function_selector("benchmark_full_trade")
}

fn get_last_result_u128_selector() -> Felt {
    function_selector("get_last_result_u128")
}

fn get_last_result_i128_selector() -> Felt {
    function_selector("get_last_result_i128")
}

fn get_last_result_felt_selector() -> Felt {
    function_selector("get_last_result_felt")
}

/// Check if a function selector is supported by this contract.
pub fn supports_selector(selector: Felt) -> bool {
    selector == benchmark_division_selector()
        || selector == benchmark_multiplication_selector()
        || selector == benchmark_addition_selector()
        || selector == benchmark_subtraction_selector()
        || selector == benchmark_fee_calculation_selector()
        || selector == benchmark_signed_division_selector()
        || selector == benchmark_signed_multiplication_selector()
        || selector == benchmark_events_selector()
        || selector == benchmark_assertions_selector()
        || selector == benchmark_full_trade_selector()
        || selector == get_last_result_u128_selector()
        || selector == get_last_result_i128_selector()
        || selector == get_last_result_felt_selector()
}

/// Execute a function on this contract.
pub fn execute<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    if selector == benchmark_division_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_division(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_multiplication_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_multiplication(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_addition_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_addition(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_subtraction_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_subtraction(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_fee_calculation_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_fee_calculation(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_signed_division_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_signed_division(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_signed_multiplication_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_signed_multiplication(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_events_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_events(state, contract_address, caller, iterations, &mut ctx)?;
    } else if selector == benchmark_assertions_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_assertions(state, contract_address, iterations, &mut ctx)?;
    } else if selector == benchmark_full_trade_selector() {
        let iterations = parse_u128_from_calldata(calldata, 0)?;
        functions::execute_benchmark_full_trade(state, contract_address, iterations, &mut ctx)?;
    } else if selector == get_last_result_u128_selector() {
        functions::execute_get_last_result_u128(state, contract_address, &mut ctx)?;
    } else if selector == get_last_result_i128_selector() {
        functions::execute_get_last_result_i128(state, contract_address, &mut ctx)?;
    } else if selector == get_last_result_felt_selector() {
        functions::execute_get_last_result_felt(state, contract_address, &mut ctx)?;
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

/// Parse u128 from calldata at given index.
fn parse_u128_from_calldata(calldata: &[Felt], index: usize) -> Result<u128, ExecutionError> {
    if calldata.len() <= index {
        return Err(ExecutionError::ExecutionFailed(format!(
            "Calldata too short: expected at least {} elements, got {}",
            index + 1,
            calldata.len()
        )));
    }

    // Cairo u128 is stored as a single felt252
    let felt = calldata[index];

    // Convert felt to u128
    let bytes = felt.to_bytes_be();

    // Check that the value fits in u128 (top 16 bytes should be zero)
    for i in 0..16 {
        if bytes[i] != 0 {
            return Err(ExecutionError::ExecutionFailed(format!(
                "Value too large for u128: {:#x}",
                felt
            )));
        }
    }

    // Extract lower 16 bytes as u128
    let mut u128_bytes = [0u8; 16];
    u128_bytes.copy_from_slice(&bytes[16..32]);
    Ok(u128::from_be_bytes(u128_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_supports_benchmark_division_selector() {
        assert!(supports_selector(benchmark_division_selector()));
    }

    #[test]
    fn test_supports_benchmark_full_trade_selector() {
        assert!(supports_selector(benchmark_full_trade_selector()));
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
    fn test_parse_u128_from_calldata() {
        let calldata = vec![Felt::from(100u64)];
        let result = parse_u128_from_calldata(&calldata, 0);
        assert_eq!(result.unwrap(), 100);
    }

    #[test]
    fn test_parse_u128_from_empty_calldata() {
        let calldata: Vec<Felt> = vec![];
        let result = parse_u128_from_calldata(&calldata, 0);
        assert!(result.is_err());
    }
}
