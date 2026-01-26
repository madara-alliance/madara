//! Function implementations for MathBenchmark contract.
//!
//! This is where Rust's native integer operations should significantly
//! outperform Cairo's felt252 field operations, especially for division.

use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::event_selector;
use crate::types::ContractAddress;

use super::layout::*;

// ============================================================================
// Constants - Must match Cairo contract exactly
// ============================================================================

/// Fixed-point multiplier (8 decimals): 1.0 = 100_000_000
const MULTIPLIER: u128 = 100_000_000;
const MULTIPLIER_I128: i128 = 100_000_000;

/// Realistic trading values
const PRICE: u128 = 4_500_000_000_000; // $45,000.00 (BTC price)
const SIZE: u128 = 100_000_000; // 1.0 units
const FEE_RATE: u128 = 30_000; // 0.03% (3 bps)
const FUNDING_RATE: u128 = 100_000; // 0.1%
const INITIAL_BALANCE: u128 = 10_000_000_000_000; // $100,000.00

/// Signed versions for PnL calculations
const PRICE_I128: i128 = 4_500_000_000_000;
const SIZE_I128: i128 = 100_000_000;

// ============================================================================
// Event selectors
// ============================================================================

fn benchmark_completed_selector() -> Felt {
    event_selector("BenchmarkCompleted")
}

fn iteration_event_selector() -> Felt {
    event_selector("IterationEvent")
}

// ============================================================================
// Helper: Convert i128 to Felt (matching Cairo's i128 -> felt252 conversion)
// ============================================================================

fn i128_to_felt(value: i128) -> Felt {
    if value >= 0 {
        Felt::from(value as u128)
    } else {
        // Negative values: use two's complement in the field
        // Cairo represents negative i128 as PRIME - |value|
        // But for felt252, we just convert directly
        Felt::from(value)
    }
}

fn u128_to_felt(value: u128) -> Felt {
    Felt::from(value)
}

// ============================================================================
// Division Benchmark (u128)
// Pattern: (price * size) / MULTIPLIER
// This is where Rust should be MUCH faster - native CPU division vs field ops
// ============================================================================

pub fn execute_benchmark_division<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut result: u128 = 0;

    for i in 0..iterations {
        // Simulate position value calculation: (price * size) / MULTIPLIER
        let price = PRICE + i;
        let notional = price * SIZE;
        let value = notional / MULTIPLIER; // Native CPU division!
        result += value;
    }

    // Store result
    ctx.storage_write(contract, *LAST_RESULT_U128_KEY, u128_to_felt(result));

    // Return result (u128 as single felt)
    ctx.set_retdata(vec![u128_to_felt(result)]);

    Ok(())
}

// ============================================================================
// Multiplication Benchmark (u128)
// Pattern: price * fee_rate
// ============================================================================

pub fn execute_benchmark_multiplication<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut result: u128 = 0;

    for i in 0..iterations {
        // Simulate fee calculation: price * fee_rate
        let price = PRICE + i;
        let fee_component = price * FEE_RATE; // Native multiplication
        result += fee_component;
    }

    ctx.storage_write(contract, *LAST_RESULT_U128_KEY, u128_to_felt(result));
    ctx.set_retdata(vec![u128_to_felt(result)]);

    Ok(())
}

// ============================================================================
// Addition Benchmark (u128)
// Pattern: Accumulating balances
// ============================================================================

pub fn execute_benchmark_addition<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut result: u128 = INITIAL_BALANCE;

    for i in 0..iterations {
        let deposit = SIZE + i;
        result += deposit; // Native addition
    }

    ctx.storage_write(contract, *LAST_RESULT_U128_KEY, u128_to_felt(result));
    ctx.set_retdata(vec![u128_to_felt(result)]);

    Ok(())
}

// ============================================================================
// Subtraction Benchmark (u128)
// Pattern: Balance deductions
// ============================================================================

pub fn execute_benchmark_subtraction<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut result: u128 = INITIAL_BALANCE * 1000;

    for i in 0..iterations {
        let withdrawal = SIZE + (i % 100);
        result -= withdrawal; // Native subtraction
    }

    ctx.storage_write(contract, *LAST_RESULT_U128_KEY, u128_to_felt(result));
    ctx.set_retdata(vec![u128_to_felt(result)]);

    Ok(())
}

// ============================================================================
// Combined Fee Calculation (mul + div)
// Pattern: fee = (notional * fee_rate) / MULTIPLIER
// ============================================================================

pub fn execute_benchmark_fee_calculation<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut total_fees: u128 = 0;

    for i in 0..iterations {
        let price = PRICE + i;
        let notional = (price * SIZE) / MULTIPLIER;
        let fee = (notional * FEE_RATE) / MULTIPLIER;
        total_fees += fee;
    }

    ctx.storage_write(contract, *LAST_RESULT_U128_KEY, u128_to_felt(total_fees));
    ctx.set_retdata(vec![u128_to_felt(total_fees)]);

    Ok(())
}

// ============================================================================
// Signed Division (i128) - For PnL calculations
// Pattern: realized_pnl = (exit_price - entry_price) * size / MULTIPLIER
// ============================================================================

pub fn execute_benchmark_signed_division<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut result: i128 = 0;

    for i in 0..iterations {
        let i_i128 = i as i128;
        let exit_price: i128 = PRICE_I128 + (i_i128 * 1000);
        let entry_price: i128 = PRICE_I128;
        let price_diff: i128 = exit_price - entry_price;
        let pnl: i128 = (price_diff * SIZE_I128) / MULTIPLIER_I128; // Native signed division!
        result += pnl;
    }

    let result_felt = i128_to_felt(result);
    ctx.storage_write(contract, *LAST_RESULT_I128_KEY, result_felt);
    ctx.set_retdata(vec![result_felt]);

    Ok(())
}

// ============================================================================
// Signed Multiplication (i128) - For funding calculations
// ============================================================================

pub fn execute_benchmark_signed_multiplication<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut result: i128 = 0;
    let funding_rate_i128 = FUNDING_RATE as i128;

    for i in 0..iterations {
        let i_i128 = i as i128;
        let position_value: i128 = PRICE_I128 + (i_i128 * 100);
        let funding: i128 = position_value * funding_rate_i128;
        result += funding;
    }

    let result_felt = i128_to_felt(result);
    ctx.storage_write(contract, *LAST_RESULT_I128_KEY, result_felt);
    ctx.set_retdata(vec![result_felt]);

    Ok(())
}

// ============================================================================
// Event Emission Benchmark
// Measures event emission overhead
// ============================================================================

pub fn execute_benchmark_events<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    _caller: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    for i in 0..iterations {
        // Emit IterationEvent per iteration
        // keys: [selector, iteration (indexed)]
        // data: [value]
        ctx.emit_event(vec![iteration_event_selector(), u128_to_felt(i)], vec![u128_to_felt(i)]);
    }

    ctx.storage_write(contract, *EVENT_COUNT_KEY, u128_to_felt(iterations));

    // Emit completion event
    // keys: [selector, operation (indexed)]
    // data: [iterations, final_result]
    ctx.emit_event(
        vec![benchmark_completed_selector(), Felt::from_bytes_be_slice(b"events")],
        vec![u128_to_felt(iterations), u128_to_felt(iterations)],
    );

    ctx.set_retdata(vec![u128_to_felt(iterations)]);

    Ok(())
}

// ============================================================================
// Assertions Benchmark
// Measures comparison operation overhead
// ============================================================================

pub fn execute_benchmark_assertions<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut pass_count: u128 = 0;

    for i in 0..iterations {
        let a = PRICE + i;
        let b = PRICE;

        // Greater than check
        assert!(a >= b, "a must be >= b");

        // Non-zero check
        assert!(a != 0, "a must be non-zero");

        // Bounds check
        assert!(a < PRICE * 2, "a must be bounded");

        pass_count += 1;
    }

    ctx.storage_write(contract, *LAST_RESULT_U128_KEY, u128_to_felt(pass_count));
    ctx.set_retdata(vec![u128_to_felt(pass_count)]);

    Ok(())
}

// ============================================================================
// Full Trade Simulation
// Combines all operations like settle_trade_v3
// ============================================================================

pub fn execute_benchmark_full_trade<S: StateReader>(
    _state: &S,
    contract: ContractAddress,
    iterations: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut total_pnl: i128 = 0;
    let mut total_fees: u128 = 0;

    for i in 0..iterations {
        let i_i128 = i as i128;

        // 1. Calculate position value (mul + div)
        let trade_price: i128 = PRICE_I128 + (i_i128 * 1000);
        let position_value: i128 = (trade_price * SIZE_I128) / MULTIPLIER_I128;

        // 2. Calculate entry cost
        let entry_price: i128 = PRICE_I128;
        let entry_cost: i128 = (entry_price * SIZE_I128) / MULTIPLIER_I128;

        // 3. Calculate PnL (subtraction)
        let pnl: i128 = position_value - entry_cost;
        total_pnl += pnl;

        // 4. Calculate fee (mul + div)
        let notional_abs: u128 = if position_value >= 0 { position_value as u128 } else { (-position_value) as u128 };
        let fee: u128 = (notional_abs * FEE_RATE) / MULTIPLIER;
        total_fees += fee;

        // 5. Assertions (health checks)
        assert!(notional_abs > 0, "notional must be positive");
    }

    // Store combined result
    let total_fees_i128: i128 = total_fees as i128;
    let final_result: i128 = total_pnl - total_fees_i128;
    let result_felt = i128_to_felt(final_result);

    ctx.storage_write(contract, *LAST_RESULT_I128_KEY, result_felt);

    // Emit completion event
    ctx.emit_event(
        vec![benchmark_completed_selector(), Felt::from_bytes_be_slice(b"full_trade")],
        vec![u128_to_felt(iterations), result_felt],
    );

    ctx.set_retdata(vec![result_felt]);

    Ok(())
}

// ============================================================================
// Getters
// ============================================================================

pub fn execute_get_last_result_u128<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let value = ctx.storage_read(state, contract, *LAST_RESULT_U128_KEY)?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

pub fn execute_get_last_result_i128<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let value = ctx.storage_read(state, contract, *LAST_RESULT_I128_KEY)?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

pub fn execute_get_last_result_felt<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let value = ctx.storage_read(state, contract, *LAST_RESULT_FELT_KEY)?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::mock::MockStateReader;

    #[test]
    fn test_benchmark_division_deterministic() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let mut ctx1 = ExecutionContext::new();
        execute_benchmark_division(&state, contract, 10, &mut ctx1).unwrap();
        let result1 = ctx1.build_result();

        let mut ctx2 = ExecutionContext::new();
        execute_benchmark_division(&state, contract, 10, &mut ctx2).unwrap();
        let result2 = ctx2.build_result();

        assert_eq!(result1.call_result.retdata, result2.call_result.retdata);
    }

    #[test]
    fn test_benchmark_fee_calculation() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let mut ctx = ExecutionContext::new();
        execute_benchmark_fee_calculation(&state, contract, 5, &mut ctx).unwrap();
        let result = ctx.build_result();

        // Should have a result
        assert!(!result.call_result.retdata.is_empty());
        // Should have stored the result
        assert!(!result.state_diff.storage_updates.is_empty());
    }

    #[test]
    fn test_benchmark_signed_division() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let mut ctx = ExecutionContext::new();
        execute_benchmark_signed_division(&state, contract, 10, &mut ctx).unwrap();
        let result = ctx.build_result();

        // Should produce a result
        assert!(!result.call_result.retdata.is_empty());
    }

    #[test]
    fn test_benchmark_events_emits_correct_count() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));
        let caller = ContractAddress(Felt::from(999u64));

        let mut ctx = ExecutionContext::new();
        execute_benchmark_events(&state, contract, caller, 5, &mut ctx).unwrap();
        let result = ctx.build_result();

        // Should emit 5 iteration events + 1 completion event = 6 total
        assert_eq!(result.call_result.events.len(), 6);
    }

    #[test]
    fn test_benchmark_full_trade() {
        let state = MockStateReader::new();
        let contract = ContractAddress(Felt::from(1u64));

        let mut ctx = ExecutionContext::new();
        execute_benchmark_full_trade(&state, contract, 10, &mut ctx).unwrap();
        let result = ctx.build_result();

        // Should have result
        assert!(!result.call_result.retdata.is_empty());
        // Should emit completion event
        assert_eq!(result.call_result.events.len(), 1);
    }
}
