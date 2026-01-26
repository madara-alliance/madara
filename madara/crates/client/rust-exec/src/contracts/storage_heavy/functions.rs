use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::types::{ContractAddress, StorageKey};

use super::{
    map_address_key, map_u128_key, nested_map_key, operation_complete_selector, storage_write_batch_selector,
    ACCOUNT_BALANCES_BASE, ACCOUNT_COUNTERS_BASE, NESTED_MAP_1_BASE, NESTED_MAP_2_BASE, TOTAL_OPERATIONS_KEY,
    TOTAL_WRITES_KEY, VALUE_MAP_1_BASE, VALUE_MAP_2_BASE, VALUE_MAP_3_BASE, VALUE_MAP_4_BASE, VALUE_MAP_5_BASE,
};

/// Execute massive_storage_write(base_value: u128, iteration_count: u32)
/// Performs 152 storage writes across multiple maps and emits 9 events
pub fn execute_massive_storage_write<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // Parse arguments
    if calldata.len() != 2 {
        return Err(ExecutionError::ExecutionFailed(format!(
            "massive_storage_write expects 2 arguments, got {}",
            calldata.len()
        )));
    }

    // base_value: u128 (low, high not used since we use u128 directly)
    let base_value_low = calldata[0]
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("base_value overflow".to_string()))?;
    let base_value: u128 = base_value_low;

    // iteration_count is ignored in the Cairo implementation (loops are hardcoded)

    let mut write_count: u128 = 0;

    // BATCH 1: Write to value_map_1 (20 writes)
    for i in 0..20u128 {
        let key = map_u128_key(*VALUE_MAP_1_BASE, i);
        let value = Felt::from(base_value + i);
        ctx.storage_write(contract, StorageKey(key), value);
        write_count += 1;
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(1u32), Felt::from(20u32)], // batch_number=1, writes_count=20
    );

    // BATCH 2: Write to value_map_2 (20 writes)
    for j in 0..20u128 {
        let key = map_u128_key(*VALUE_MAP_2_BASE, j);
        let value = Felt::from(base_value + j + 1000);
        ctx.storage_write(contract, StorageKey(key), value);
        write_count += 1;
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(2u32), Felt::from(20u32)],
    );

    // BATCH 3: Write to value_map_3 (20 writes)
    for k in 0..20u128 {
        let key = map_u128_key(*VALUE_MAP_3_BASE, k);
        let value = Felt::from(base_value + k + 2000);
        ctx.storage_write(contract, StorageKey(key), value);
        write_count += 1;
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(3u32), Felt::from(20u32)],
    );

    // BATCH 4: Write to value_map_4 (20 writes)
    for m in 0..20u128 {
        let key = map_u128_key(*VALUE_MAP_4_BASE, m);
        let value = Felt::from(base_value + m + 3000);
        ctx.storage_write(contract, StorageKey(key), value);
        write_count += 1;
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(4u32), Felt::from(20u32)],
    );

    // BATCH 5: Write to value_map_5 (20 writes)
    for n in 0..20u128 {
        let key = map_u128_key(*VALUE_MAP_5_BASE, n);
        let value = Felt::from(base_value + n + 4000);
        ctx.storage_write(contract, StorageKey(key), value);
        write_count += 1;
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(5u32), Felt::from(20u32)],
    );

    // BATCH 6: Write to nested_map_1 (25 writes - 5x5 grid)
    for x in 0..5u128 {
        for y in 0..5u128 {
            let key = nested_map_key(*NESTED_MAP_1_BASE, x, y);
            let value = Felt::from(base_value + x * 10 + y);
            ctx.storage_write(contract, StorageKey(key), value);
            write_count += 1;
        }
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(6u32), Felt::from(25u32)],
    );

    // BATCH 7: Write to nested_map_2 (25 writes - 5x5 grid)
    for p in 0..5u128 {
        for q in 0..5u128 {
            let key = nested_map_key(*NESTED_MAP_2_BASE, p, q);
            let value = Felt::from(base_value + p * 100 + q * 10);
            ctx.storage_write(contract, StorageKey(key), value);
            write_count += 1;
        }
    }

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(7u32), Felt::from(25u32)],
    );

    // BATCH 8: Update account storage (2 writes)
    let balance_key = map_address_key(*ACCOUNT_BALANCES_BASE, caller.0);
    let current_balance_felt = ctx.storage_read(state, contract, StorageKey(balance_key))?;
    let current_balance: u128 = current_balance_felt
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("balance overflow".to_string()))?;
    ctx.storage_write(contract, StorageKey(balance_key), Felt::from(current_balance + base_value));
    write_count += 1;

    let counter_key = map_address_key(*ACCOUNT_COUNTERS_BASE, caller.0);
    let current_counter_felt = ctx.storage_read(state, contract, StorageKey(counter_key))?;
    let current_counter: u32 = current_counter_felt
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("counter overflow".to_string()))?;
    ctx.storage_write(contract, StorageKey(counter_key), Felt::from(current_counter + 1));
    write_count += 1;

    ctx.emit_event(
        vec![storage_write_batch_selector()],
        vec![Felt::from(8u32), Felt::from(2u32)],
    );

    // BATCH 9: Update global counters (2 writes)
    let current_total_felt = ctx.storage_read(state, contract, StorageKey(*TOTAL_WRITES_KEY))?;
    let current_total: u128 = current_total_felt
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("total_writes overflow".to_string()))?;
    ctx.storage_write(contract, StorageKey(*TOTAL_WRITES_KEY), Felt::from(current_total + write_count));

    let current_ops_felt = ctx.storage_read(state, contract, StorageKey(*TOTAL_OPERATIONS_KEY))?;
    let current_ops: u128 = current_ops_felt
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("total_operations overflow".to_string()))?;
    let new_ops = current_ops + 1;
    ctx.storage_write(contract, StorageKey(*TOTAL_OPERATIONS_KEY), Felt::from(new_ops));

    write_count += 2;

    // Emit final OperationComplete event
    ctx.emit_event(
        vec![operation_complete_selector()],
        vec![Felt::from(write_count), Felt::from(new_ops)],
    );

    // No return data
    ctx.set_retdata(vec![]);
    Ok(())
}

/// Execute get_value_map1(key: u128) -> u128
pub fn execute_get_value_map1<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    if calldata.len() != 1 {
        return Err(ExecutionError::ExecutionFailed(format!(
            "get_value_map1 expects 1 argument, got {}",
            calldata.len()
        )));
    }

    let key: u128 = calldata[0]
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("key overflow".to_string()))?;

    let storage_key = map_u128_key(*VALUE_MAP_1_BASE, key);
    let value = ctx.storage_read(state, contract, StorageKey(storage_key))?;

    ctx.set_retdata(vec![value]);
    Ok(())
}

/// Execute get_value_map2(key: u128) -> u128
pub fn execute_get_value_map2<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    if calldata.len() != 1 {
        return Err(ExecutionError::ExecutionFailed(format!(
            "get_value_map2 expects 1 argument, got {}",
            calldata.len()
        )));
    }

    let key: u128 = calldata[0]
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("key overflow".to_string()))?;

    let storage_key = map_u128_key(*VALUE_MAP_2_BASE, key);
    let value = ctx.storage_read(state, contract, StorageKey(storage_key))?;

    ctx.set_retdata(vec![value]);
    Ok(())
}

/// Execute get_value_map3(key: u128) -> u128
pub fn execute_get_value_map3<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    if calldata.len() != 1 {
        return Err(ExecutionError::ExecutionFailed(format!(
            "get_value_map3 expects 1 argument, got {}",
            calldata.len()
        )));
    }

    let key: u128 = calldata[0]
        .to_biguint()
        .try_into()
        .map_err(|_| ExecutionError::ExecutionFailed("key overflow".to_string()))?;

    let storage_key = map_u128_key(*VALUE_MAP_3_BASE, key);
    let value = ctx.storage_read(state, contract, StorageKey(storage_key))?;

    ctx.set_retdata(vec![value]);
    Ok(())
}

/// Execute get_total_writes() -> u128
pub fn execute_get_total_writes<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let value = ctx.storage_read(state, contract, StorageKey(*TOTAL_WRITES_KEY))?;
    ctx.set_retdata(vec![value]);
    Ok(())
}
