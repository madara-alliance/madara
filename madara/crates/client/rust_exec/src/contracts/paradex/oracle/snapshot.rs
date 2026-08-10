use starknet_types_core::felt::Felt;

use super::{funding_index_data_base, latest_tick_data_base};
use crate::contracts::paradex::oracle::codec::felt_to_i128;
use crate::contracts::paradex::schema::oracle_layout as layout;
use crate::contracts::paradex::schema::oracle_types::TickData;
use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::storage_key_with_offset;
use crate::core::types::ContractAddress;

pub(super) fn set_prices_and_funding_snapshot<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    latest_snapshot_id: Felt,
    new_prices: &[TickData],
    new_indices: &[TickData],
) -> Result<(), ExecutionError> {
    if let Some(tick) = new_prices.first().or_else(|| new_indices.first()) {
        assert_timestamp_monotonic(ctx, state, contract, tick.last_updated_timestamp)?;
        ctx.storage_write(contract, *layout::LATEST_UPDATED_TIMESTAMP_BASE, tick.last_updated_timestamp);
    }

    for tick in new_prices {
        set_value(ctx, state, contract, tick, false)?;
    }

    for tick in new_indices {
        set_funding_index(ctx, state, contract, tick)?;
    }

    ctx.storage_write(contract, *layout::LATEST_SNAPSHOT_ID_BASE, latest_snapshot_id);
    Ok(())
}

fn assert_timestamp_monotonic<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    timestamp: Felt,
) -> Result<(), ExecutionError> {
    let current_timestamp = ctx.storage_read(state, contract, *layout::LATEST_UPDATED_TIMESTAMP_BASE)?;
    if timestamp < current_timestamp {
        return Err(ExecutionError::ExecutionFailed("TIMESTAMP_TOO_OLD".to_string()));
    }
    Ok(())
}

fn set_value<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    tick: &TickData,
    allow_zero: bool,
) -> Result<(), ExecutionError> {
    assert_tick_decimals(tick)?;
    let asset_value = felt_to_i128(tick.asset_value);
    if (!allow_zero && asset_value <= 0) || (allow_zero && asset_value < 0) {
        return Err(ExecutionError::ExecutionFailed(if allow_zero {
            "price must be non-negative".to_string()
        } else {
            "price must be positive".to_string()
        }));
    }

    let base = latest_tick_data_base(ctx, state, contract, tick.asset_key)?;
    let stored_asset_key = ctx.storage_read(state, contract, base)?;
    if stored_asset_key != tick.asset_key {
        ctx.storage_write(contract, base, tick.asset_key);
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
        ctx.storage_write(contract, storage_key_with_offset(base, 2), tick.decimals);
    } else {
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
    }

    Ok(())
}

fn set_funding_index<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    tick: &TickData,
) -> Result<(), ExecutionError> {
    assert_tick_decimals(tick)?;

    let base = funding_index_data_base(ctx, state, contract, tick.asset_key)?;
    let stored_asset_key = ctx.storage_read(state, contract, base)?;
    if stored_asset_key != tick.asset_key {
        ctx.storage_write(contract, base, tick.asset_key);
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
        ctx.storage_write(contract, storage_key_with_offset(base, 2), tick.decimals);
    } else {
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
    }

    Ok(())
}

fn assert_tick_decimals(tick: &TickData) -> Result<(), ExecutionError> {
    if tick.decimals != Felt::from(8u64) {
        return Err(ExecutionError::ExecutionFailed("INVALID_DECIMALS".to_string()));
    }
    Ok(())
}
