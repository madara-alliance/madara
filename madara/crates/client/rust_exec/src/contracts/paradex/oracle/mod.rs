//! ParaclearOracle contract implementation.

mod codec;
mod names;
mod selectors;
mod snapshot;

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::contracts::paradex::schema::oracle_layout as layout;
use crate::contracts::paradex::schema::oracle_types::TickData;
use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::{sn_keccak, storage_key_for_map_with_base_named, storage_key_with_offset};
use crate::core::types::{ContractAddress, ExecutionResult, StorageKey};

use codec::{decode_felt_array, decode_tick_data_array, take_felt};
pub(crate) use names::PRECOMPUTED_NAMES;
pub(crate) use selectors::FUNCTION_NAMES;
use selectors::{
    decimals_selector, get_function_name as selector_function_name, get_funding_index_selector,
    get_latest_snapshot_id_selector, get_name_selector, get_value_selector, get_values_with_funding_indices_selector,
    get_version_selector, set_prices_and_funding_snapshot_selector,
};
use snapshot::set_prices_and_funding_snapshot;

const LATEST_TICK_DATA_INDEX: usize = 0;
const FUNDING_INDEX_DATA_INDEX: usize = 1;

/// Supported Paradex ParaclearOracle class hash.
pub const CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x00049e91ccb24fcf4acec4a24896092d9387a97865dcb0e6f98503399564b452");

static LATEST_TICK_DATA_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[LATEST_TICK_DATA_INDEX].as_bytes()));
static FUNDING_INDEX_DATA_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[FUNDING_INDEX_DATA_INDEX].as_bytes()));

pub fn supports_class_hash(class_hash: Felt) -> bool {
    class_hash == CLASS_HASH
}

pub fn supports_selector(selector: Felt) -> bool {
    selector == get_value_selector()
        || selector == get_values_with_funding_indices_selector()
        || selector == get_funding_index_selector()
        || selector == set_prices_and_funding_snapshot_selector()
        || selector == get_latest_snapshot_id_selector()
        || selector == decimals_selector()
        || selector == get_name_selector()
        || selector == get_version_selector()
}

pub fn get_function_name(selector: Felt) -> Option<String> {
    selector_function_name(selector).map(str::to_string)
}

pub fn execute<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    _caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    let mut ctx = ExecutionContext::new();

    if selector == get_value_selector() {
        let market = take_felt(calldata)?;
        let tick = read_tick_data(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![tick.asset_key, tick.asset_value, tick.decimals, tick.last_updated_timestamp]);
    } else if selector == get_values_with_funding_indices_selector() {
        let (markets, _rest) = decode_felt_array(calldata)?;
        let mut values = Vec::with_capacity(markets.len());
        let mut indices = Vec::with_capacity(markets.len());
        for m in markets {
            let tick = read_tick_data(&mut ctx, state, contract, m)?;
            values.push(tick.asset_value);
            indices.push(read_funding_index(&mut ctx, state, contract, m)?);
        }
        let settlement_price = read_settlement_token_price(&mut ctx, state, contract, settlement_token_asset_key())?;
        let mut ret = Vec::new();
        ret.push(Felt::from(values.len() as u64));
        ret.extend(values);
        ret.push(Felt::from(indices.len() as u64));
        ret.extend(indices);
        ret.push(settlement_price);
        ctx.set_retdata(ret);
    } else if selector == get_funding_index_selector() {
        let market = take_felt(calldata)?;
        let idx = read_funding_index(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![idx]);
    } else if selector == set_prices_and_funding_snapshot_selector() {
        let latest_snapshot_id = take_felt(calldata)?;
        let (new_prices, rest) = decode_tick_data_array(&calldata[1..])?;
        let (new_indices, rest) = decode_tick_data_array(rest)?;
        if !rest.is_empty() {
            return Err(ExecutionError::ExecutionFailed("unexpected trailing calldata".to_string()));
        }
        set_prices_and_funding_snapshot(&mut ctx, state, contract, latest_snapshot_id, &new_prices, &new_indices)?;
    } else if selector == get_latest_snapshot_id_selector() {
        let value = read_latest_snapshot_id(&mut ctx, state, contract)?;
        ctx.set_retdata(vec![value]);
    } else if selector == decimals_selector() {
        ctx.set_retdata(vec![Felt::from(8u64)]);
    } else if selector == get_name_selector() {
        ctx.set_retdata(vec![short_string("ParaclearOracle")]);
    } else if selector == get_version_selector() {
        ctx.set_retdata(vec![short_string("1.0.9")]);
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

fn settlement_token_asset_key() -> Felt {
    // From oracle interface: SETTLEMENT_TOKEN_ASSET_KEY
    short_string("USDC")
}

fn short_string(s: &str) -> Felt {
    crate::core::storage::short_string_to_felt(s)
}

pub fn read_tick_data(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<TickData, ExecutionError> {
    let base = latest_tick_data_base(ctx, state, contract, market)?;
    let asset_key = ctx.storage_read(state, contract, base)?;
    let asset_value = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    let decimals = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let last_updated_timestamp = ctx.storage_read(state, contract, *layout::LATEST_UPDATED_TIMESTAMP_BASE)?;

    Ok(TickData { asset_key, asset_value, decimals, last_updated_timestamp })
}

pub fn read_funding_index(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<Felt, ExecutionError> {
    let base = funding_index_data_base(ctx, state, contract, market)?;
    let asset_value = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    Ok(asset_value)
}

pub fn read_latest_snapshot_id(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
) -> Result<Felt, ExecutionError> {
    Ok(ctx.storage_read(state, contract, *layout::LATEST_SNAPSHOT_ID_BASE)?)
}

pub fn read_settlement_token_price(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    settlement_token_asset_key: Felt,
) -> Result<Felt, ExecutionError> {
    let base = latest_tick_data_base(ctx, state, contract, settlement_token_asset_key)?;
    let asset_value = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    Ok(asset_value)
}

pub(super) fn latest_tick_data_base(
    _ctx: &mut ExecutionContext,
    _state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<StorageKey, ExecutionError> {
    thread_local! {
        static LATEST_TICK_BASE_CACHE: RefCell<HashMap<(ContractAddress, Felt), StorageKey>> =
            RefCell::new(HashMap::new());
    }
    if let Some(base) = LATEST_TICK_BASE_CACHE.with(|cache| cache.borrow().get(&(contract, market)).copied()) {
        return Ok(base);
    }

    let base = storage_key_for_map_with_base_named(*LATEST_TICK_DATA_BASE, market, "latest_tick_data");
    LATEST_TICK_BASE_CACHE.with(|cache| cache.borrow_mut().insert((contract, market), base));
    Ok(base)
}

pub(super) fn funding_index_data_base(
    _ctx: &mut ExecutionContext,
    _state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<StorageKey, ExecutionError> {
    thread_local! {
        static FUNDING_BASE_CACHE: RefCell<HashMap<(ContractAddress, Felt), StorageKey>> =
            RefCell::new(HashMap::new());
    }
    if let Some(base) = FUNDING_BASE_CACHE.with(|cache| cache.borrow().get(&(contract, market)).copied()) {
        return Ok(base);
    }

    let base = storage_key_for_map_with_base_named(*FUNDING_INDEX_DATA_BASE, market, "funding_index_data");
    FUNDING_BASE_CACHE.with(|cache| cache.borrow_mut().insert((contract, market), base));
    Ok(base)
}

#[allow(dead_code)]
fn read_felt_at(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    base: StorageKey,
    offset: u8,
) -> Result<Felt, ExecutionError> {
    Ok(ctx.storage_read(state, contract, storage_key_with_offset(base, offset))?)
}
