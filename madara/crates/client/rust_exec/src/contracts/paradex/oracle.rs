//! ParaclearOracle contract implementation.

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::context::ExecutionContext;
use crate::contracts::paradex_codegen::oracle_layout as layout;
use crate::contracts::paradex_codegen::oracle_types::TickData;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::{function_selector, sn_keccak, storage_key_for_map_with_base_named, storage_key_with_offset};
use crate::types::{ContractAddress, ExecutionResult, StorageKey};

// TODO: These base keys use sn_keccak and are then hashed with Pedersen via
// `storage_key_for_map_with_base_named` in `latest_tick_data_base()` / `funding_index_data_base()`.
// However, Cairo 1.0 contracts using `Map<K, V>` storage compute map keys with Poseidon hash,
// not Pedersen. The auto-generated `oracle_layout.rs` has the correct Poseidon-based functions
// (`latest_tick_data_key`, `funding_index_data_key`) but they are not used here.
// If the deployed oracle contract was compiled with a Cairo 1.0 compiler, these keys will read
// from the wrong storage slots. Verify which hash the deployed contract uses and switch to
// `layout::latest_tick_data_key(market)` / `layout::funding_index_data_key(market)` if needed.
static LATEST_TICK_DATA_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("latest_tick_data".as_bytes()));
static FUNDING_INDEX_DATA_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("funding_index_data".as_bytes()));

pub fn supports_selector(selector: Felt) -> bool {
    selector == get_value_selector()
        || selector == get_values_with_funding_indices_selector()
        || selector == get_funding_index_selector()
        || selector == get_latest_snapshot_id_selector()
        || selector == decimals_selector()
        || selector == get_version_selector()
        || selector == get_name_selector()
        || selector == set_prices_and_funding_snapshot_selector()
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
    } else if selector == get_version_selector() {
        Some("get_version".to_string())
    } else if selector == get_name_selector() {
        Some("get_name".to_string())
    } else if selector == set_prices_and_funding_snapshot_selector() {
        Some("set_prices_and_funding_snapshot".to_string())
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
            values.push(read_tick_data_asset_value(&mut ctx, state, contract, m)?);
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
    } else if selector == get_version_selector() {
        ctx.set_retdata(vec![short_string("1.0.7")]);
    } else if selector == get_name_selector() {
        ctx.set_retdata(vec![short_string("ParaclearOracle")]);
    } else if selector == set_prices_and_funding_snapshot_selector() {
        execute_set_prices_and_funding_snapshot(&mut ctx, state, contract, calldata)?;
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

fn get_version_selector() -> Felt {
    function_selector("get_version")
}

fn get_name_selector() -> Felt {
    function_selector("get_name")
}

fn set_prices_and_funding_snapshot_selector() -> Felt {
    function_selector("set_prices_and_funding_snapshot")
}

const DECIMALS: Felt = Felt::from_hex_unchecked("0x8");

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

/// Read only the asset_value field from latest_tick_data for a market.
/// This matches Cairo's `self.latest_tick_data.entry(market).asset_value.read()` which reads
/// a single storage slot, rather than reading the entire TickData struct.
fn read_tick_data_asset_value(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<Felt, ExecutionError> {
    let base = latest_tick_data_base(ctx, state, contract, market)?;
    Ok(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?)
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

// ── set_prices_and_funding_snapshot implementation ───────────────────────────

/// Decode an `Array<TickData>` from calldata.
///
/// Starknet ABI encodes arrays of structs as:
/// `[len, item_0_fields..., item_1_fields..., ...]`
/// where `len` is the number of structs, not the number of flattened felts.
/// Each `TickData` contributes 4 felts:
/// `[asset_key, asset_value, decimals, last_updated_timestamp]`.
fn decode_tick_data_array(input: &[Felt]) -> Result<(Vec<TickData>, &[Felt]), ExecutionError> {
    let len = take_felt(input)?;
    let tick_count = felt_to_u32(len)? as usize;
    let flat_len = tick_count
        .checked_mul(4)
        .ok_or_else(|| ExecutionError::ExecutionFailed("TickData array length overflow".to_string()))?;
    if input.len() < 1 + flat_len {
        return Err(ExecutionError::ExecutionFailed(format!(
            "TickData array underflow: expected {} felts for {} TickData items, got {}",
            flat_len,
            tick_count,
            input.len().saturating_sub(1)
        )));
    }
    let items = &input[1..1 + flat_len];
    let ticks: Vec<TickData> = items
        .chunks_exact(4)
        .map(|chunk| TickData {
            asset_key: chunk[0],
            asset_value: chunk[1],
            decimals: chunk[2],
            last_updated_timestamp: chunk[3],
        })
        .collect();
    Ok((ticks, &input[1 + flat_len..]))
}

/// Mirrors Cairo's `_assert_timestamp_monotonic`: new timestamp must be >= current.
fn assert_timestamp_monotonic(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    new_timestamp: Felt,
) -> Result<(), ExecutionError> {
    let current = ctx.storage_read(state, contract, *layout::LATEST_UPDATED_TIMESTAMP_BASE)?;
    // Compare as u256 (same as Cairo: `let timestamp_u256: u256 = timestamp.into()`)
    if new_timestamp < current {
        return Err(ExecutionError::ExecutionFailed("TIMESTAMP_TOO_OLD".to_string()));
    }
    Ok(())
}

/// Mirrors Cairo's `_set_value`: write price data for a single asset.
/// If the asset_key slot already contains this asset_key, only update asset_value.
/// Otherwise, write the full TickDataPrice struct (asset_key, asset_value, decimals).
fn set_value(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    tick: &TickData,
) -> Result<(), ExecutionError> {
    if tick.decimals != DECIMALS {
        return Err(ExecutionError::ExecutionFailed("INVALID_DECIMALS".to_string()));
    }
    let base = latest_tick_data_base(ctx, state, contract, tick.asset_key)?;
    let existing_key = ctx.storage_read(state, contract, base)?;
    if existing_key != tick.asset_key {
        // New entry: write full struct
        ctx.storage_write(contract, base, tick.asset_key);
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
        ctx.storage_write(contract, storage_key_with_offset(base, 2), tick.decimals);
    } else {
        // Existing entry: update only asset_value
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
    }
    Ok(())
}

/// Mirrors Cairo's `_set_funding_index`: write funding index for a single asset.
/// Same logic as `set_value` but writes to funding_index_data storage.
fn set_funding_index(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    tick: &TickData,
) -> Result<(), ExecutionError> {
    if tick.decimals != DECIMALS {
        return Err(ExecutionError::ExecutionFailed("INVALID_DECIMALS".to_string()));
    }
    let base = funding_index_data_base(ctx, state, contract, tick.asset_key)?;
    let existing_key = ctx.storage_read(state, contract, base)?;
    if existing_key != tick.asset_key {
        // New entry: write full struct
        ctx.storage_write(contract, base, tick.asset_key);
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
        ctx.storage_write(contract, storage_key_with_offset(base, 2), tick.decimals);
    } else {
        // Existing entry: update only asset_value
        ctx.storage_write(contract, storage_key_with_offset(base, 1), tick.asset_value);
    }
    Ok(())
}

/// Execute `set_prices_and_funding_snapshot(latest_snapshot_id, new_prices, new_indices)`.
///
/// Calldata layout: [snapshot_id, prices_len, prices..., indices_len, indices...]
/// where each TickData is 4 felts: [asset_key, asset_value, decimals, last_updated_timestamp].
fn execute_set_prices_and_funding_snapshot<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    calldata: &[Felt],
) -> Result<(), ExecutionError> {
    if calldata.is_empty() {
        return Err(ExecutionError::ExecutionFailed("calldata underflow".to_string()));
    }
    let snapshot_id = calldata[0];
    let rest = &calldata[1..];

    let (new_prices, rest) = decode_tick_data_array(rest)?;
    let (new_indices, _rest) = decode_tick_data_array(rest)?;

    // Set last updated timestamp from the first price or first index
    if let Some(first_price) = new_prices.first() {
        assert_timestamp_monotonic(ctx, state, contract, first_price.last_updated_timestamp)?;
        ctx.storage_write(contract, *layout::LATEST_UPDATED_TIMESTAMP_BASE, first_price.last_updated_timestamp);
    } else if let Some(first_index) = new_indices.first() {
        assert_timestamp_monotonic(ctx, state, contract, first_index.last_updated_timestamp)?;
        ctx.storage_write(contract, *layout::LATEST_UPDATED_TIMESTAMP_BASE, first_index.last_updated_timestamp);
    }

    for tick in &new_prices {
        set_value(ctx, state, contract, tick)?;
    }

    for tick in &new_indices {
        set_funding_index(ctx, state, contract, tick)?;
    }

    ctx.storage_write(contract, *layout::LATEST_SNAPSHOT_ID_BASE, snapshot_id);

    Ok(())
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
