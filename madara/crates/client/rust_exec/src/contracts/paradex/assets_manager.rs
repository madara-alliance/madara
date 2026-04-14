//! AssetsManager read-only helpers for Paraclear.

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::{
    function_selector, short_string_to_felt, sn_keccak, storage_key_for_map_poseidon_with_base_named,
    storage_key_for_substorage_map_poseidon_add_with_var_named,
    storage_key_for_substorage_map_poseidon_hash_with_var_named,
    storage_key_for_substorage_map_poseidon_with_var_named, storage_key_with_offset,
};
use crate::types::{ContractAddress, ExecutionResult};

use crate::contracts::paradex_codegen::assets_manager_layout;
use crate::contracts::paradex_codegen::assets_manager_types::{
    NamedToken, PerpetualAsset, PerpetualMarginParams, PerpetualOptionAsset, SpotAsset,
};

pub const ASSET_KIND_UNSUPPORTED: Felt = Felt::ZERO;

static SPOT_ASSET_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak("spot_asset".as_bytes()));
static SPOT_ASSET_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak("spot.spot_asset".as_bytes()));
static FUTURE_ASSET_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak("perpetual_future_asset".as_bytes()));
static FUTURE_ASSET_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak("future.perpetual_future_asset".as_bytes()));
static OPTION_ASSET_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak("perpetual_option_asset".as_bytes()));
static OPTION_ASSET_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak("option.perpetual_option_asset".as_bytes()));

pub fn supports_selector(selector: Felt) -> bool {
    selector == get_asset_kind_selector() || selector == get_base_token_asset_selector()
}

pub fn get_function_name(selector: Felt) -> Option<String> {
    if selector == get_asset_kind_selector() {
        Some("get_asset_kind".to_string())
    } else if selector == get_base_token_asset_selector() {
        Some("get_base_token_asset".to_string())
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

    if selector == get_asset_kind_selector() {
        let market = take_felt(calldata)?;
        let kind = get_asset_kind(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![kind]);
    } else if selector == get_base_token_asset_selector() {
        let market = take_felt(calldata)?;
        let token = get_base_token_asset(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![token.token_address.0, token.token_name]);
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

pub fn asset_kind_future() -> Felt {
    short_string_to_felt("FUTURE")
}
pub fn asset_kind_option() -> Felt {
    short_string_to_felt("OPTION")
}
pub fn asset_kind_spot() -> Felt {
    short_string_to_felt("SPOT")
}

pub fn get_asset_kind(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<Felt, ExecutionError> {
    if market == Felt::ZERO {
        return Ok(ASSET_KIND_UNSUPPORTED);
    }

    if is_future_supported(ctx, state, contract, market)? {
        return Ok(asset_kind_future());
    }
    if is_option_supported(ctx, state, contract, market)? {
        return Ok(asset_kind_option());
    }
    if is_spot_supported(ctx, state, contract, market)? {
        return Ok(asset_kind_spot());
    }

    Ok(ASSET_KIND_UNSUPPORTED)
}

pub fn get_base_token_asset(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<NamedToken, ExecutionError> {
    let spot = read_spot_asset(ctx, state, contract, market)?;
    if spot.base_token_address.0 == Felt::ZERO {
        return Err(ExecutionError::ExecutionFailed("Spot: asset not found".to_string()));
    }
    Ok(NamedToken { token_address: spot.base_token_address, token_name: spot.base_token_name })
}

fn is_spot_supported(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<bool, ExecutionError> {
    let spot = read_spot_asset(ctx, state, contract, market)?;
    Ok(spot.market == market)
}

fn is_future_supported(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<bool, ExecutionError> {
    let asset = read_perpetual_asset(ctx, state, contract, market)?;
    Ok(asset.market == market)
}

fn is_option_supported(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<bool, ExecutionError> {
    let asset = read_perpetual_option_asset(ctx, state, contract, market)?;
    Ok(asset.market == market)
}

fn read_spot_asset(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<SpotAsset, ExecutionError> {
    read_spot_asset_with_fallback(ctx, state, contract, market)
}

fn read_perpetual_asset(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<PerpetualAsset, ExecutionError> {
    read_perpetual_asset_with_fallback(ctx, state, contract, market)
}

fn read_perpetual_option_asset(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<PerpetualOptionAsset, ExecutionError> {
    read_perpetual_option_asset_with_fallback(ctx, state, contract, market)
}

fn read_spot_asset_with_fallback(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<SpotAsset, ExecutionError> {
    let base_key = *assets_manager_layout::SPOT_BASE;
    let candidates = [("spot_asset", *SPOT_ASSET_VAR), ("spot.spot_asset", *SPOT_ASSET_DOTTED_VAR)];
    for (name, var) in candidates {
        let direct = storage_key_for_map_poseidon_with_base_named(var, market, name);
        let sub = storage_key_for_substorage_map_poseidon_with_var_named(base_key, var, market, name);
        let sub_add = storage_key_for_substorage_map_poseidon_add_with_var_named(base_key, var, market, name);
        let sub_hash = storage_key_for_substorage_map_poseidon_hash_with_var_named(base_key, var, market, name);
        for (_, base) in
            [("direct", direct), ("substorage", sub), ("substorage_add", sub_add), ("substorage_hash", sub_hash)]
        {
            let market_felt = ctx.storage_read(state, contract, base)?;
            let base_token_address =
                ContractAddress(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?);
            let base_token_name = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
            let quote_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
            if market_felt != Felt::ZERO {
                return Ok(SpotAsset { market: market_felt, base_token_address, base_token_name, quote_asset });
            }
        }
    }
    Ok(SpotAsset {
        market: Felt::ZERO,
        base_token_address: ContractAddress(Felt::ZERO),
        base_token_name: Felt::ZERO,
        quote_asset: Felt::ZERO,
    })
}

fn read_perpetual_asset_with_fallback(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<PerpetualAsset, ExecutionError> {
    let base_key = *assets_manager_layout::FUTURE_BASE;
    let candidates =
        [("perpetual_future_asset", *FUTURE_ASSET_VAR), ("future.perpetual_future_asset", *FUTURE_ASSET_DOTTED_VAR)];
    for (name, var) in candidates {
        let direct = storage_key_for_map_poseidon_with_base_named(var, market, name);
        let sub = storage_key_for_substorage_map_poseidon_with_var_named(base_key, var, market, name);
        let sub_add = storage_key_for_substorage_map_poseidon_add_with_var_named(base_key, var, market, name);
        let sub_hash = storage_key_for_substorage_map_poseidon_hash_with_var_named(base_key, var, market, name);
        for (_, base) in
            [("direct", direct), ("substorage", sub), ("substorage_add", sub_add), ("substorage_hash", sub_hash)]
        {
            let market_felt = ctx.storage_read(state, contract, base)?;
            let base_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
            let quote_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
            let tick_size = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
            let imf_base = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
            let imf_factor = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
            let mmf_factor = ctx.storage_read(state, contract, storage_key_with_offset(base, 6))?;
            let imf_shift = ctx.storage_read(state, contract, storage_key_with_offset(base, 7))?;
            if market_felt != Felt::ZERO {
                return Ok(PerpetualAsset {
                    market: market_felt,
                    base_asset,
                    quote_asset,
                    tick_size,
                    margin_params: PerpetualMarginParams { imf_base, imf_factor, mmf_factor, imf_shift },
                });
            }
        }
    }
    Ok(PerpetualAsset {
        market: Felt::ZERO,
        base_asset: Felt::ZERO,
        quote_asset: Felt::ZERO,
        tick_size: Felt::ZERO,
        margin_params: PerpetualMarginParams {
            imf_base: Felt::ZERO,
            imf_factor: Felt::ZERO,
            mmf_factor: Felt::ZERO,
            imf_shift: Felt::ZERO,
        },
    })
}

fn read_perpetual_option_asset_with_fallback(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<PerpetualOptionAsset, ExecutionError> {
    let base_key = *assets_manager_layout::OPTION_BASE;
    let candidates =
        [("perpetual_option_asset", *OPTION_ASSET_VAR), ("option.perpetual_option_asset", *OPTION_ASSET_DOTTED_VAR)];
    for (name, var) in candidates {
        let direct = storage_key_for_map_poseidon_with_base_named(var, market, name);
        let sub = storage_key_for_substorage_map_poseidon_with_var_named(base_key, var, market, name);
        let sub_add = storage_key_for_substorage_map_poseidon_add_with_var_named(base_key, var, market, name);
        let sub_hash = storage_key_for_substorage_map_poseidon_hash_with_var_named(base_key, var, market, name);
        for (_, base) in
            [("direct", direct), ("substorage", sub), ("substorage_add", sub_add), ("substorage_hash", sub_hash)]
        {
            let market_felt = ctx.storage_read(state, contract, base)?;
            let base_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
            let quote_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
            let tick_size = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
            let option_type = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
            let strike = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
            if market_felt != Felt::ZERO {
                return Ok(PerpetualOptionAsset {
                    market: market_felt,
                    base_asset,
                    quote_asset,
                    tick_size,
                    option_type,
                    strike,
                });
            }
        }
    }
    Ok(PerpetualOptionAsset {
        market: Felt::ZERO,
        base_asset: Felt::ZERO,
        quote_asset: Felt::ZERO,
        tick_size: Felt::ZERO,
        option_type: Felt::ZERO,
        strike: Felt::ZERO,
    })
}

fn get_asset_kind_selector() -> Felt {
    function_selector("get_asset_kind")
}

fn get_base_token_asset_selector() -> Felt {
    function_selector("get_base_token_asset")
}

fn take_felt(input: &[Felt]) -> Result<Felt, ExecutionError> {
    if input.is_empty() {
        return Err(ExecutionError::ExecutionFailed("calldata underflow".to_string()));
    }
    Ok(input[0])
}
