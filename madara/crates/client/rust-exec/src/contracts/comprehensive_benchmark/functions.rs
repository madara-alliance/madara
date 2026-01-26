//! Function implementations for ComprehensiveBenchmark contract.

use num_bigint::BigUint;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::types::{ContractAddress, StorageKey};

use super::*;

// Helper function: Read-then-write to match Cairo's .write() semantics
// CRITICAL: Cairo's .write() always reads the old value first before writing.
// This ensures state diff logic matches:
// - If old != new: include in diff (value changed)
// - If old == new: exclude from diff (no change)
// - If writing 0 without reading: excluded (unread zero) - CAUSES MISMATCH!
fn storage_write_matching_cairo<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
    key: StorageKey,
    value: Felt,
) -> Result<(), ExecutionError> {
    // Read current value (mimics Cairo's implicit read in .write())
    let old_value = ctx.storage_read(state, contract, key)?;

    tracing::info!(
        "🔍 storage_write_matching_cairo: key={:#x} old={:#x} new={:#x}",
        key.0, old_value, value
    );

    // Write new value
    ctx.storage_write(contract, key, value);

    Ok(())
}

// Helper function to update u128 counters (total_operations, total_hashes_computed, total_events_emitted)
// Matches StorageHeavy pattern: simple u128, single storage slot
fn update_u128_counter<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
    key: StorageKey,
    increment: u128,
) -> Result<(), ExecutionError> {
    // Read current value
    let current = ctx.storage_read(state, contract, key)?;
    let current_u128: u128 = current.to_biguint().try_into().unwrap_or(0);

    // Add increment with wrapping (overflow → wrap around)
    let new_value = current_u128.wrapping_add(increment);

    // Write new value (read already done above)
    ctx.storage_write(contract, key, Felt::from(new_value));

    Ok(())
}

// Constants from Cairo contract
const MULTIPLIER: u128 = 100_000_000; // 8 decimals
const PRICE: u128 = 4_500_000_000_000; // $45,000.00
const SIZE: u128 = 100_000_000; // 1.0 units
const FEE_RATE: u128 = 30_000; // 0.03%
const PRICE_I128: i128 = 4_500_000_000_000;
const SIZE_I128: i128 = 100_000_000;

// ============================================================================
// A. STORAGE BENCHMARKS (6 functions)
// ============================================================================

pub fn execute_benchmark_storage_read<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;
    for _ in 0..iterations {
        let value = ctx.storage_read(state, contract, StorageKey(*SIMPLE_COUNTER_KEY));
        result = value?.to_biguint().try_into().unwrap_or(0);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "storage_read", iterations, result.into(), iterations);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_storage_write<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    tracing::info!("🔧 benchmark_storage_write called: iterations={}", iterations);

    for i in 0..iterations {
        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_COUNTER_KEY), Felt::from(i))?;
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(iterations))?;

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);

    // Emit event
    emit_benchmark_complete(ctx, "storage_write", iterations, iterations.into(), iterations);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

pub fn execute_benchmark_map_read<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;
    for i in 0..iterations {
        let key = i % 100;
        let storage_key = map_u128_key(*SIMPLE_MAP_BASE, key);
        let value = ctx.storage_read(state, contract, StorageKey(storage_key));
        let value_u128: u128 = value?.to_biguint().try_into().unwrap_or(0);
        result = result.wrapping_add(value_u128);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "map_read", iterations, result.into(), iterations);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_map_write<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    for i in 0..iterations {
        let key = i % 100;
        let storage_key = map_u128_key(*SIMPLE_MAP_BASE, key);
        storage_write_matching_cairo(state, contract, ctx, StorageKey(storage_key), Felt::from(i))?;
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(iterations))?;

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "map_write", iterations, iterations.into(), iterations);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

pub fn execute_benchmark_nested_map_read<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;
    for i in 0..iterations {
        let key1 = i % 10;
        let key2 = (i / 10) % 10;
        let storage_key = nested_map_key(*NESTED_MAP_BASE, key1, key2);
        let value = ctx.storage_read(state, contract, StorageKey(storage_key));
        let value_u128: u128 = value?.to_biguint().try_into().unwrap_or(0);
        result = result.wrapping_add(value_u128);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "nested_map_read", iterations, result.into(), iterations);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_nested_map_write<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    for i in 0..iterations {
        let key1 = i % 10;
        let key2 = (i / 10) % 10;
        let storage_key = nested_map_key(*NESTED_MAP_BASE, key1, key2);
        storage_write_matching_cairo(state, contract, ctx, StorageKey(storage_key), Felt::from(i))?;
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(iterations))?;

    // Update total operations
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_OPERATIONS_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "nested_map_write", iterations, iterations.into(), iterations);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

// ============================================================================
// B. HASHING BENCHMARKS (3 functions)
// ============================================================================

pub fn execute_benchmark_pedersen_hashing<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let seed = Felt::from(12345u128);
    let mut current_hash = seed;

    for i in 0..iterations {
        current_hash = Pedersen::hash(&current_hash, &Felt::from(i));
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_FELT_KEY), current_hash)?;

    // Update total hashes
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_HASHES_COMPUTED_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "pedersen_hashing", iterations, current_hash, iterations);

    ctx.set_retdata(vec![current_hash]);
    Ok(())
}

pub fn execute_benchmark_pedersen_parallel<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let base = Felt::from(54321u128);
    let mut last_hash = Felt::ZERO;

    for i in 0..iterations {
        last_hash = Pedersen::hash(&base, &Felt::from(i));
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_FELT_KEY), last_hash)?;

    // Update total hashes
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_HASHES_COMPUTED_KEY), iterations);


    // Emit event
    emit_benchmark_complete(ctx, "pedersen_parallel", iterations, last_hash, iterations);

    ctx.set_retdata(vec![last_hash]);
    Ok(())
}

pub fn execute_benchmark_poseidon_hashing<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    for i in 0..iterations {
        // Emit HashComputed event (uses Poseidon internally)
        emit_hash_computed(ctx, i, "poseidon");
    }

    // Update total events
    update_u128_counter(state, contract, ctx, StorageKey(*TOTAL_EVENTS_EMITTED_KEY), iterations);

    // Emit final event
    emit_benchmark_complete(ctx, "poseidon_hashing", iterations, iterations.into(), iterations);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

// ============================================================================
// C. MATH BENCHMARKS (8 functions)
// ============================================================================

pub fn execute_benchmark_math_division<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;
    for i in 0..iterations {
        let price = PRICE.wrapping_add(i);
        let notional = price.wrapping_mul(SIZE);
        let value = notional / MULTIPLIER;
        result = result.wrapping_add(value);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations


    // Emit event
    emit_benchmark_complete(ctx, "math_division", iterations, result.into(), iterations * 3);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_math_multiplication<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;
    for i in 0..iterations {
        let price = PRICE.wrapping_add(i);
        let fee_component = price.wrapping_mul(FEE_RATE);
        result = result.wrapping_add(fee_component);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_multiplication", iterations, result.into(), iterations * 2);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_math_addition<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 1000000;
    for i in 0..iterations {
        let deposit = SIZE.wrapping_add(i);
        result = result.wrapping_add(deposit);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_addition", iterations, result.into(), iterations * 2);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_math_subtraction<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 1000000000000;
    for i in 0..iterations {
        let withdrawal = SIZE.wrapping_add(i % 100);
        result = result.wrapping_sub(withdrawal);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_subtraction", iterations, result.into(), iterations * 3);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_math_signed_division<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: i128 = 0;
    for i in 0..iterations {
        let i_i128 = i as i128;
        let exit_price = PRICE_I128 + (i_i128 * 1000);
        let entry_price = PRICE_I128;
        let price_diff = exit_price - entry_price;
        let pnl = (price_diff * SIZE_I128) / 100_000_000;
        result = result + pnl;
    }

    let result_felt = Felt::from(result);
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_I128_KEY), result_felt)?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_signed_div", iterations, result_felt, iterations * 5);

    ctx.set_retdata(vec![result_felt]);
    Ok(())
}

pub fn execute_benchmark_math_signed_multiplication<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: i128 = 0;
    let funding_rate: i128 = 100_000;

    for i in 0..iterations {
        let i_i128 = i as i128;
        let position_value = PRICE_I128 + (i_i128 * 100);
        let funding = position_value * funding_rate;
        result = result + funding;
    }

    let result_felt = Felt::from(result);
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_I128_KEY), result_felt)?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_signed_mul", iterations, result_felt, iterations * 3);

    ctx.set_retdata(vec![result_felt]);
    Ok(())
}

pub fn execute_benchmark_math_combined_ops<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut total_fees: u128 = 0;
    for i in 0..iterations {
        let price = PRICE.wrapping_add(i);
        let notional = (price.wrapping_mul(SIZE)) / MULTIPLIER;
        let fee = (notional.wrapping_mul(FEE_RATE)) / MULTIPLIER;
        total_fees = total_fees.wrapping_add(fee);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(total_fees))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_combined", iterations, total_fees.into(), iterations * 5);

    ctx.set_retdata(vec![Felt::from(total_fees)]);
    Ok(())
}

pub fn execute_benchmark_math_signed_combined<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut total_pnl: i128 = 0;
    let mut total_fees: u128 = 0;

    for i in 0..iterations {
        let i_i128 = i as i128;
        let trade_price = PRICE_I128 + (i_i128 * 1000);
        let position_value = (trade_price * SIZE_I128) / 100_000_000;
        let entry_cost = (PRICE_I128 * SIZE_I128) / 100_000_000;
        let pnl = position_value - entry_cost;
        total_pnl = total_pnl + pnl;

        let notional_abs: u128 = if position_value >= 0 {
            position_value as u128
        } else {
            (-position_value) as u128
        };
        let fee = (notional_abs * FEE_RATE) / MULTIPLIER;
        total_fees = total_fees + fee;
    }

    let total_fees_i128 = total_fees as i128;
    let final_result = total_pnl - total_fees_i128;
    let result_felt = Felt::from(final_result);

    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_I128_KEY), result_felt)?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "math_signed_combined", iterations, result_felt, iterations * 12);

    ctx.set_retdata(vec![result_felt]);
    Ok(())
}

// ============================================================================
// D. CONTROL FLOW BENCHMARKS (3 functions)
// ============================================================================

pub fn execute_benchmark_loops<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let counter = iterations; // Simple counting

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(counter))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "loops", iterations, counter.into(), iterations);

    ctx.set_retdata(vec![Felt::from(counter)]);
    Ok(())
}

pub fn execute_benchmark_conditionals<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;
    for i in 0..iterations {
        let value = i % 10;
        if value < 3 {
            result += 1;
        } else if value < 6 {
            result += 2;
        } else if value < 9 {
            result += 3;
        } else {
            result += 4;
        }
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "conditionals", iterations, result.into(), iterations * 4);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_assertions<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut pass_count: u128 = 0;
    for i in 0..iterations {
        let a = PRICE + i;
        let b = PRICE;

        // Assertions (would panic in Cairo if failed, we just verify)
        if a >= b && a != 0 && a < PRICE * 2 {
            pass_count += 1;
        }
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(pass_count))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "assertions", iterations, pass_count.into(), iterations * 3);

    ctx.set_retdata(vec![Felt::from(pass_count)]);
    Ok(())
}

// ============================================================================
// E. COMPLEX PATTERN BENCHMARKS (5 functions)
// ============================================================================

pub fn execute_benchmark_struct_read<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;

    // AccountState struct has 4 fields, balance is first
    let balance_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);

    for _ in 0..iterations {
        let balance = ctx.storage_read(state, contract, StorageKey(balance_key));
        let balance_u128: u128 = balance?.to_biguint().try_into().unwrap_or(0);
        result = result.wrapping_add(balance_u128);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "struct_read", iterations, result.into(), iterations);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_struct_write<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    // AccountState struct: balance, position, realized_pnl, margin_ratio
    let base_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);

    for i in 0..iterations {
        // Write ALL 4 struct fields (matching Cairo implementation)
        // Cairo creates: AccountState { balance: i, position: i, realized_pnl: 0, margin_ratio: 15_000_000 }

        // Field 0: balance
        storage_write_matching_cairo(state, contract, ctx,
            StorageKey(base_key), Felt::from(i))?;

        // Field 1: position
        storage_write_matching_cairo(state, contract, ctx,
            StorageKey(base_key + Felt::ONE), Felt::from(i))?;

        // Field 2: realized_pnl
        storage_write_matching_cairo(state, contract, ctx,
            StorageKey(base_key + Felt::TWO), Felt::ZERO)?;

        // Field 3: margin_ratio (15,000,000 in Cairo)
        storage_write_matching_cairo(state, contract, ctx,
            StorageKey(base_key + Felt::THREE), Felt::from(15_000_000u128))?;
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(iterations))?;

    // Emit event
    emit_benchmark_complete(ctx, "struct_write", iterations, iterations.into(), iterations);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

pub fn execute_benchmark_multi_map_coordination<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let market_id = Felt::ONE;
    let mut result: u128 = 0;

    for i in 0..iterations {
        // Read from 3 maps
        let account_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);
        let balance = ctx.storage_read(state, contract, StorageKey(account_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        let address_key = map_address_key(*ADDRESS_MAP_BASE, caller.0);
        let address_value = ctx.storage_read(state, contract, StorageKey(address_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        let position_key = map_address_felt_key(*POSITIONS_BASE, caller.0, market_id);
        let entry_price = ctx.storage_read(state, contract, StorageKey(position_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        // Compute
        let total = balance.wrapping_add(address_value).wrapping_add(entry_price);
        result = result.wrapping_add(total);

        // Write to 3 maps
        storage_write_matching_cairo(state, contract, ctx, StorageKey(account_key), Felt::from(balance))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(address_key), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(position_key), Felt::from(entry_price))?;
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "multi_map_coord", iterations, result.into(), iterations * 6);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_referential_reads<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let mut result: u128 = 0;

    for _ in 0..iterations {
        // Chain: account -> referrer -> fee
        let referrer_key = map_address_key(*ACCOUNT_REFERRERS_BASE, caller.0);
        let referrer = ctx.storage_read(state, contract, StorageKey(referrer_key))?;

        let fee_key = map_address_key(*REFERRER_FEES_BASE, referrer);
        let fee = ctx.storage_read(state, contract, StorageKey(fee_key));

        let fee_u128: u128 = fee?.to_biguint().try_into().unwrap_or(0);
        result = result.wrapping_add(fee_u128);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "referential_reads", iterations, result.into(), iterations * 2);

    ctx.set_retdata(vec![Felt::from(result)]);
    Ok(())
}

pub fn execute_benchmark_batch_operations<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    for i in 0..iterations {
        // Write to 10 different storage locations
        let key1 = map_u128_key(*SIMPLE_MAP_BASE, i % 100);
        let key2 = map_u128_key(*SIMPLE_MAP_BASE, (i + 1) % 100);
        let key3 = map_u128_key(*SIMPLE_MAP_BASE, (i + 2) % 100);
        let key4 = map_u128_key(*SIMPLE_MAP_BASE, (i + 3) % 100);
        let key5 = map_u128_key(*SIMPLE_MAP_BASE, (i + 4) % 100);

        storage_write_matching_cairo(state, contract, ctx, StorageKey(key1), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(key2), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(key3), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(key4), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(key5), Felt::from(i))?;

        let nested1 = nested_map_key(*NESTED_MAP_BASE, i % 10, (i / 10) % 10);
        let nested2 = nested_map_key(*NESTED_MAP_BASE, (i + 1) % 10, (i / 10) % 10);
        let nested3 = nested_map_key(*NESTED_MAP_BASE, (i + 2) % 10, (i / 10) % 10);

        storage_write_matching_cairo(state, contract, ctx, StorageKey(nested1), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(nested2), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(nested3), Felt::from(i))?;

        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_COUNTER_KEY), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_ACCUMULATOR_KEY), Felt::from(i))?;
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(iterations))?;

    // Update total operations
    // Emit event
    emit_benchmark_complete(ctx, "batch_operations", iterations, iterations.into(), iterations * 10);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

// ============================================================================
// F. END-TO-END BENCHMARKS (3 functions)
// ============================================================================

pub fn execute_benchmark_simple_transaction<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    for i in 0..iterations {
        // 1 read
        let current = ctx.storage_read(state, contract, StorageKey(*SIMPLE_COUNTER_KEY))?
            .to_biguint().try_into().unwrap_or(0u128);

        // 1 computation
        let new_value = current + 1;

        // 1 write
        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_COUNTER_KEY), Felt::from(new_value))?;

        // 1 event
        emit_operation_batch(ctx, i.try_into().unwrap_or(0), 1);
    }

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(iterations))?;

    // Update total operations

    // Emit final event
    emit_benchmark_complete(ctx, "simple_transaction", iterations, iterations.into(), iterations * 4);

    ctx.set_retdata(vec![Felt::from(iterations)]);
    Ok(())
}

pub fn execute_benchmark_medium_transaction<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    // V3 update: Use u256 for result accumulator to prevent overflow
    let mut result: BigUint = BigUint::from(0u128);

    for i in 0..iterations {
        // 5 reads
        let val1_key = map_u128_key(*SIMPLE_MAP_BASE, i % 10);
        let val1 = ctx.storage_read(state, contract, StorageKey(val1_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        let val2_key = map_u128_key(*SIMPLE_MAP_BASE, (i + 1) % 10);
        let val2 = ctx.storage_read(state, contract, StorageKey(val2_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        let val3_key = nested_map_key(*NESTED_MAP_BASE, i % 5, (i / 5) % 5);
        let val3 = ctx.storage_read(state, contract, StorageKey(val3_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        let state_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);
        let balance = ctx.storage_read(state, contract, StorageKey(state_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        let addr_key = map_address_key(*ADDRESS_MAP_BASE, caller.0);
        let address_val = ctx.storage_read(state, contract, StorageKey(addr_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        // 10 computations
        let price = PRICE.wrapping_add(i);
        let notional = price.wrapping_mul(SIZE);
        let value = notional / MULTIPLIER;
        let fee = (value.wrapping_mul(FEE_RATE)) / MULTIPLIER;
        let total = val1.wrapping_add(val2).wrapping_add(val3).wrapping_add(balance).wrapping_add(address_val);
        let final_value = total.wrapping_add(value).wrapping_sub(fee);
        // V3 update: Accumulate in u256
        result = result + BigUint::from(final_value);

        // 5 writes
        storage_write_matching_cairo(state, contract, ctx, StorageKey(val1_key), Felt::from(final_value))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(val2_key), Felt::from(final_value))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(val3_key), Felt::from(final_value))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(addr_key), Felt::from(final_value))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_COUNTER_KEY), Felt::from(i))?;

        // 3 events (simplified - 2 events)
        emit_operation_batch(ctx, i.try_into().unwrap_or(0), 20);
        emit_hash_computed(ctx, i, "medium_tx");
    }

    // V3 update: Truncate u256 result to u128 for storage/return
    let result_u128: u128 = result.try_into().unwrap_or(u128::MAX);

    // Write result
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_U128_KEY), Felt::from(result_u128))?;

    // Update total operations

    // Emit final event
    emit_benchmark_complete(ctx, "medium_transaction", iterations, result_u128.into(), iterations * 23);

    ctx.set_retdata(vec![Felt::from(result_u128)]);
    Ok(())
}

pub fn execute_benchmark_heavy_transaction<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let iterations: u128 = calldata[0].to_biguint().try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("iterations".to_string()))?;

    let market_id = Felt::ONE;
    // V3 update: Use felt252 for total_pnl to prevent overflow
    let mut total_pnl: Felt = Felt::ZERO;
    // V4 update: Use u128 for hash_count (matches StorageHeavy pattern)
    let mut hash_count: u128 = 0;

    for i in 0..iterations {
        // 12 map reads (complex state loading)
        let account_key = map_address_key(*ACCOUNT_STATES_BASE, caller.0);

        // Read ALL 4 AccountState fields (Cairo reads entire struct)
        let balance = ctx.storage_read(state, contract, StorageKey(account_key))?;
        let position = ctx.storage_read(state, contract, StorageKey(account_key + Felt::ONE))?;
        let realized_pnl = ctx.storage_read(state, contract, StorageKey(account_key + Felt::TWO))?;
        let margin_ratio = ctx.storage_read(state, contract, StorageKey(account_key + Felt::THREE))?;

        let position_key = map_address_felt_key(*POSITIONS_BASE, caller.0, market_id);

        // Read ALL 3 PositionData fields (Cairo reads entire struct)
        let pos_size = ctx.storage_read(state, contract, StorageKey(position_key))?;
        let entry_price = ctx.storage_read(state, contract, StorageKey(position_key + Felt::ONE))?;
        let last_funding_paid = ctx.storage_read(state, contract, StorageKey(position_key + Felt::TWO))?;

        // Read 3 simple_map values (val1, val2, val3)
        let val1 = ctx.storage_read(state, contract, StorageKey(map_u128_key(*SIMPLE_MAP_BASE, i % 20)))?
            .to_biguint().try_into().unwrap_or(0u128);
        let val2 = ctx.storage_read(state, contract, StorageKey(map_u128_key(*SIMPLE_MAP_BASE, (i + 1) % 20)))?
            .to_biguint().try_into().unwrap_or(0u128);
        let val3 = ctx.storage_read(state, contract, StorageKey(map_u128_key(*SIMPLE_MAP_BASE, (i + 2) % 20)))?
            .to_biguint().try_into().unwrap_or(0u128);

        // Read 3 nested_map values (val4, val5, val6)
        let val4 = ctx.storage_read(state, contract,
            StorageKey(nested_map_key(*NESTED_MAP_BASE, i % 5, (i / 5) % 5)))?
            .to_biguint().try_into().unwrap_or(0u128);
        let val5 = ctx.storage_read(state, contract,
            StorageKey(nested_map_key(*NESTED_MAP_BASE, (i + 1) % 5, (i / 5) % 5)))?
            .to_biguint().try_into().unwrap_or(0u128);
        let val6 = ctx.storage_read(state, contract,
            StorageKey(nested_map_key(*NESTED_MAP_BASE, (i + 2) % 5, (i / 5) % 5)))?
            .to_biguint().try_into().unwrap_or(0u128);

        let referrer_key = map_address_key(*ACCOUNT_REFERRERS_BASE, caller.0);
        let referrer = ctx.storage_read(state, contract, StorageKey(referrer_key))?;

        let fee_key = map_address_key(*REFERRER_FEES_BASE, referrer);
        let _fee_config = ctx.storage_read(state, contract, StorageKey(fee_key))?
            .to_biguint().try_into().unwrap_or(0u128);

        // Read 2 address_map values (addr_val1, addr_val2)
        let addr_val1 = ctx.storage_read(state, contract,
            StorageKey(map_address_key(*ADDRESS_MAP_BASE, caller.0)))?
            .to_biguint().try_into().unwrap_or(0u128);
        let addr_val2 = ctx.storage_read(state, contract,
            StorageKey(map_address_key(*ADDRESS_MAP_BASE, referrer)))?
            .to_biguint().try_into().unwrap_or(0u128);

        // 30 Pedersen hashes
        let mut hash_accumulator = Felt::from(i);
        for j in 0..30 {
            hash_accumulator = Pedersen::hash(&hash_accumulator, &Felt::from(j));
            hash_count += 1;
        }

        // 50 math operations
        let i_i128 = i as i128;
        let trade_price = PRICE_I128 + (i_i128 * 1000);
        let position_value = (trade_price * SIZE_I128) / 100_000_000;
        let entry_cost = (PRICE_I128 * SIZE_I128) / 100_000_000;
        let pnl = position_value - entry_cost;
        // V3 update: Accumulate PnL as felt252
        total_pnl = total_pnl + Felt::from(pnl);

        let price = PRICE.wrapping_add(i);
        let notional = (price.wrapping_mul(SIZE)) / MULTIPLIER;
        let fee = (notional.wrapping_mul(FEE_RATE)) / MULTIPLIER;

        // Calculate total_value from all read values (matching Cairo)
        let total_value = val1.wrapping_add(val2).wrapping_add(val3)
            .wrapping_add(val4).wrapping_add(val5).wrapping_add(val6)
            .wrapping_add(addr_val1).wrapping_add(addr_val2);

        // 12 map writes (state updates)
        // AccountState struct: write back ALL 4 fields (Cairo reads entire struct and writes back)
        storage_write_matching_cairo(state, contract, ctx, StorageKey(account_key), balance)?;  // balance
        storage_write_matching_cairo(state, contract, ctx, StorageKey(account_key + Felt::ONE), position)?;  // position
        storage_write_matching_cairo(state, contract, ctx, StorageKey(account_key + Felt::TWO), realized_pnl)?;  // realized_pnl
        storage_write_matching_cairo(state, contract, ctx, StorageKey(account_key + Felt::THREE), margin_ratio)?;  // margin_ratio

        // Position struct: write back ALL 3 fields (Cairo reads entire struct and writes back)
        storage_write_matching_cairo(state, contract, ctx, StorageKey(position_key), pos_size)?;  // size
        storage_write_matching_cairo(state, contract, ctx, StorageKey(position_key + Felt::ONE), entry_price)?;  // entry_price
        storage_write_matching_cairo(state, contract, ctx, StorageKey(position_key + Felt::TWO), last_funding_paid)?;  // last_funding_paid

        // Write to 3 simple_map entries (matching Cairo: i % 20, (i+1) % 20, (i+2) % 20)
        for j in 0..3 {
            let key = map_u128_key(*SIMPLE_MAP_BASE, (i + j) % 20);
            storage_write_matching_cairo(state, contract, ctx, StorageKey(key), Felt::from(total_value))?;
        }

        // Write to 3 nested_map entries (matching Cairo)
        for j in 0..3 {
            let key = nested_map_key(*NESTED_MAP_BASE, (i + j) % 5, (i / 5) % 5);
            storage_write_matching_cairo(state, contract, ctx, StorageKey(key), Felt::from(total_value))?;
        }

        let addr_key = map_address_key(*ADDRESS_MAP_BASE, caller.0);
        storage_write_matching_cairo(state, contract, ctx, StorageKey(addr_key), Felt::from(total_value))?;

        let referrer_fee_key = map_address_key(*ADDRESS_MAP_BASE, referrer);
        storage_write_matching_cairo(state, contract, ctx, StorageKey(referrer_fee_key), Felt::from(fee))?;

        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_COUNTER_KEY), Felt::from(i))?;
        storage_write_matching_cairo(state, contract, ctx, StorageKey(*SIMPLE_ACCUMULATOR_KEY), Felt::from(total_value))?;

        // 2 events (simplified from 10)
        emit_operation_batch(ctx, i.try_into().unwrap_or(0), 110);
        emit_hash_computed(ctx, hash_count, "heavy_tx");
    }

    // Store felt252 result in LAST_RESULT_FELT_KEY
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*LAST_RESULT_FELT_KEY), total_pnl)?;

    // V4 update: Write u128 hash_count to total_hashes_computed (single slot, matches Cairo)
    storage_write_matching_cairo(state, contract, ctx, StorageKey(*TOTAL_HASHES_COMPUTED_KEY), Felt::from(hash_count))?;

    // Update total operations

    // Emit final event
    emit_benchmark_complete(ctx, "heavy_transaction", iterations, total_pnl, iterations * 120);

    ctx.set_retdata(vec![total_pnl]);
    Ok(())
}

// ============================================================================
// GETTERS
// ============================================================================

pub fn execute_get_last_result_u128<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let result = ctx.storage_read(state, contract, StorageKey(*LAST_RESULT_U128_KEY))?;
    ctx.set_retdata(vec![result]);
    Ok(())
}

pub fn execute_get_last_result_i128<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let result = ctx.storage_read(state, contract, StorageKey(*LAST_RESULT_I128_KEY))?;
    ctx.set_retdata(vec![result]);
    Ok(())
}

pub fn execute_get_total_operations<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // V4 update: total_operations is now u128 (single slot), not u256
    let value = ctx.storage_read(state, contract, StorageKey(*TOTAL_OPERATIONS_KEY))?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

// ============================================================================
// EVENT HELPERS
// ============================================================================

fn emit_benchmark_complete(
    ctx: &mut ExecutionContext,
    benchmark_name: &str,
    iterations: u128,
    final_result: Felt,
    operations_performed: u128,
) {
    // Convert benchmark name to felt252
    let name_bytes = benchmark_name.as_bytes();
    let name_felt = if name_bytes.len() <= 31 {
        Felt::from_bytes_be_slice(name_bytes)
    } else {
        Felt::from_bytes_be_slice(&name_bytes[..31])
    };

    ctx.emit_event(
        vec![benchmark_complete_selector(), name_felt],
        vec![
            Felt::from(iterations),
            final_result,
            Felt::from(operations_performed),
        ],
    );
}

fn emit_operation_batch(ctx: &mut ExecutionContext, batch_number: u32, operations_count: u32) {
    ctx.emit_event(
        vec![operation_batch_selector()],
        vec![
            Felt::from(batch_number),
            Felt::from(operations_count),
        ],
    );
}

fn emit_hash_computed(ctx: &mut ExecutionContext, hash_count: u128, operation_type: &str) {
    let type_bytes = operation_type.as_bytes();
    let type_felt = if type_bytes.len() <= 31 {
        Felt::from_bytes_be_slice(type_bytes)
    } else {
        Felt::from_bytes_be_slice(&type_bytes[..31])
    };

    ctx.emit_event(
        vec![hash_computed_selector()],
        vec![
            Felt::from(hash_count),
            type_felt,
        ],
    );
}
