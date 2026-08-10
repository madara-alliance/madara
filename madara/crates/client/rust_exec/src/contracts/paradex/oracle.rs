//! ParaclearOracle contract (read-only helpers + execution stubs).

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::context::ExecutionContext;
use crate::contracts::paradex::schema::oracle_layout as layout;
use crate::contracts::paradex::schema::oracle_types::TickData;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::{function_selector, sn_keccak, storage_key_for_map_with_base_named, storage_key_with_offset};
use crate::types::{ContractAddress, ExecutionResult, StorageKey};

/// Paradex ParaclearOracle class hash for contracts versions 1.25.1 and 1.25.3.
pub const CLASS_HASH_1_25_1_AND_1_25_3: Felt =
    Felt::from_hex_unchecked("0x00049e91ccb24fcf4acec4a24896092d9387a97865dcb0e6f98503399564b452");

static LATEST_TICK_DATA_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("latest_tick_data".as_bytes()));
static FUNDING_INDEX_DATA_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("funding_index_data".as_bytes()));

pub fn supports_class_hash(class_hash: Felt) -> bool {
    class_hash == CLASS_HASH_1_25_1_AND_1_25_3
}

pub fn supports_selector(selector: Felt) -> bool {
    selector == get_value_selector()
        || selector == get_values_with_funding_indices_selector()
        || selector == get_funding_index_selector()
        || selector == get_latest_snapshot_id_selector()
        || selector == decimals_selector()
        || selector == get_name_selector()
        || selector == get_version_selector()
}

pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == get_value_selector() {
        Some("get_value".to_string())
    } else if selector == get_values_with_funding_indices_selector() {
        Some("get_values_with_funding_indices".to_string())
    } else if selector == get_funding_index_selector() {
        Some("get_funding_index".to_string())
    } else if selector == get_latest_snapshot_id_selector() {
        Some("get_latest_snapshot_id".to_string())
    } else if selector == decimals_selector() {
        Some("decimals".to_string())
    } else if selector == get_name_selector() {
        Some("get_name".to_string())
    } else if selector == get_version_selector() {
        Some("get_version".to_string())
    } else {
        None
    }
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

fn get_value_selector() -> Felt {
    function_selector("get_value")
}

fn get_values_with_funding_indices_selector() -> Felt {
    function_selector("get_values_with_funding_indices")
}

fn get_funding_index_selector() -> Felt {
    function_selector("get_funding_index")
}

fn get_latest_snapshot_id_selector() -> Felt {
    function_selector("get_latest_snapshot_id")
}

fn decimals_selector() -> Felt {
    function_selector("decimals")
}

fn get_name_selector() -> Felt {
    function_selector("get_name")
}

fn get_version_selector() -> Felt {
    function_selector("get_version")
}

fn settlement_token_asset_key() -> Felt {
    // From oracle interface: SETTLEMENT_TOKEN_ASSET_KEY
    short_string("USDC")
}

fn short_string(s: &str) -> Felt {
    crate::storage::short_string_to_felt(s)
}

fn take_felt(input: &[Felt]) -> Result<Felt, ExecutionError> {
    if input.is_empty() {
        return Err(ExecutionError::ExecutionFailed("calldata underflow".to_string()));
    }
    Ok(input[0])
}

fn decode_felt_array(input: &[Felt]) -> Result<(Vec<Felt>, &[Felt]), ExecutionError> {
    let len = take_felt(input)?;
    let len_u32 = felt_to_u32(len)? as usize;
    if input.len() < 1 + len_u32 {
        return Err(ExecutionError::ExecutionFailed("array underflow".to_string()));
    }
    let items = input[1..1 + len_u32].to_vec();
    Ok((items, &input[1 + len_u32..]))
}

fn felt_to_u32(value: Felt) -> Result<u32, ExecutionError> {
    let bytes = value.to_bytes_be();
    if bytes[..28].iter().any(|b| *b != 0) {
        return Err(ExecutionError::ExecutionFailed("value too large for u32".to_string()));
    }
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&bytes[28..]);
    Ok(u32::from_be_bytes(arr))
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

fn latest_tick_data_base(
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

fn funding_index_data_base(
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
