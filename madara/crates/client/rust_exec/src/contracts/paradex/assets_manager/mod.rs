//! AssetsManager read-only helpers for Paraclear.

mod names;
mod selectors;

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::{
    short_string_to_felt, sn_keccak, storage_key_for_map_poseidon, storage_key_for_map_poseidon_with_base_named,
    storage_key_for_substorage_map_poseidon, storage_key_for_substorage_map_poseidon_add,
    storage_key_for_substorage_map_poseidon_add_with_var_named, storage_key_for_substorage_map_poseidon_hash,
    storage_key_for_substorage_map_poseidon_hash_with_var_named,
    storage_key_for_substorage_map_poseidon_with_var_named, storage_key_for_substorage_var_add,
    storage_key_for_substorage_var_poseidon, storage_key_with_offset,
};
use crate::core::types::{ContractAddress, ExecutionResult, StorageKey};

use crate::contracts::paradex::schema::assets_manager_layout;
use crate::contracts::paradex::schema::assets_manager_types::{
    FeeCategory, FeeWithCap, MarketFeeConfig, NamedToken, OptionAsset, OptionCrossMarginParams, OptionMarginParams,
    PerpetualAsset, PerpetualMarginParams, SpotAsset,
};

pub(crate) use names::PRECOMPUTED_NAMES;
pub(crate) use selectors::FUNCTION_NAMES;
use selectors::{
    get_asset_kind_selector, get_asset_min_size_increment_selector, get_base_token_asset_selector,
    get_function_name as selector_function_name, get_name_selector, get_version_selector, take_felt,
};

pub const ASSET_KIND_UNSUPPORTED: Felt = Felt::ZERO;

static SPOT_ASSET_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[0].as_bytes()));
static SPOT_ASSET_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[1].as_bytes()));
static FUTURE_ASSET_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[2].as_bytes()));
static FUTURE_ASSET_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[3].as_bytes()));
static OPTION_ASSET_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[4].as_bytes()));
static OPTION_ASSET_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[5].as_bytes()));
static PERP_OPTION_MARGIN_PARAMS_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[6].as_bytes()));
static PERP_OPTION_MARGIN_PARAMS_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[7].as_bytes()));
static DATED_OPTION_MARGIN_PARAMS_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[8].as_bytes()));
static DATED_OPTION_MARGIN_PARAMS_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[9].as_bytes()));
static MARKET_FEE_CONFIG_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[10].as_bytes()));
static MARKET_FEE_CONFIG_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[11].as_bytes()));
static BASE_ASSET_PERP_OPTION_FEE_CONFIG_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[12].as_bytes()));
static BASE_ASSET_PERP_OPTION_FEE_CONFIG_DOTTED_VAR: Lazy<Felt> =
    Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[13].as_bytes()));
static BASE_ASSET_DATED_OPTION_FEE_CONFIG_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[14].as_bytes()));
static BASE_ASSET_DATED_OPTION_FEE_CONFIG_DOTTED_VAR: Lazy<Felt> =
    Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[15].as_bytes()));
static SETTLEMENT_FEE_CONFIG_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[16].as_bytes()));
static SETTLEMENT_FEE_CONFIG_DOTTED_VAR: Lazy<Felt> = Lazy::new(|| sn_keccak(PRECOMPUTED_NAMES[17].as_bytes()));

pub fn supports_selector(selector: Felt) -> bool {
    selector == get_asset_kind_selector()
        || selector == get_base_token_asset_selector()
        || selector == get_asset_min_size_increment_selector()
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

    if selector == get_asset_kind_selector() {
        let market = take_felt(calldata)?;
        let kind = get_asset_kind(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![kind]);
    } else if selector == get_base_token_asset_selector() {
        let market = take_felt(calldata)?;
        let token = get_base_token_asset(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![token.token_address.0, token.token_name]);
    } else if selector == get_asset_min_size_increment_selector() {
        let market = take_felt(calldata)?;
        let min_size_increment = get_asset_min_size_increment(&mut ctx, state, contract, market)?;
        ctx.set_retdata(vec![min_size_increment]);
    } else if selector == get_name_selector() {
        ctx.set_retdata(vec![short_string_to_felt("Assets Manager")]);
    } else if selector == get_version_selector() {
        ctx.set_retdata(vec![short_string_to_felt("1.6.0")]);
    } else {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    Ok(ctx.build_result())
}

pub fn asset_kind_future() -> Felt {
    short_string_to_felt("FUTURE")
}

pub fn asset_kind_option() -> Felt {
    asset_kind_perp_option()
}

pub fn asset_kind_perp_option() -> Felt {
    short_string_to_felt("OPTION")
}

pub fn asset_kind_spot() -> Felt {
    short_string_to_felt("SPOT")
}

pub fn asset_kind_dated_option() -> Felt {
    short_string_to_felt("DATED_OPTION")
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

    let stored_kind = ctx.storage_read(state, contract, assets_manager_layout::asset_kind_key(market))?;
    if stored_kind != Felt::ZERO {
        return Ok(stored_kind);
    }

    if is_future_supported(ctx, state, contract, market)? {
        return Ok(asset_kind_future());
    }
    if let Some(kind) = get_option_kind(ctx, state, contract, market)? {
        return Ok(kind);
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
    Ok(NamedToken { token_address: spot.base_token_address, token_name: spot.base_token_name })
}

pub fn get_asset_min_size_increment(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<Felt, ExecutionError> {
    Ok(ctx.storage_read(state, contract, assets_manager_layout::asset_min_size_increment_key(market))?)
}

pub fn get_option_asset(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<OptionAsset, ExecutionError> {
    read_option_asset_with_fallback(ctx, state, contract, market)
}

pub fn get_option_margin_params(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
    base_asset: Felt,
) -> Result<OptionCrossMarginParams, ExecutionError> {
    let (plain, dotted, var, dotted_var) = if kind == asset_kind_dated_option() {
        (
            "dated_option_margin_params",
            "option.dated_option_margin_params",
            *DATED_OPTION_MARGIN_PARAMS_VAR,
            *DATED_OPTION_MARGIN_PARAMS_DOTTED_VAR,
        )
    } else {
        (
            "perpetual_option_margin_params",
            "option.perpetual_option_margin_params",
            *PERP_OPTION_MARGIN_PARAMS_VAR,
            *PERP_OPTION_MARGIN_PARAMS_DOTTED_VAR,
        )
    };
    let base = read_component_map_base(
        ctx,
        state,
        contract,
        *assets_manager_layout::OPTION_BASE,
        plain,
        dotted,
        var,
        dotted_var,
        base_asset,
    )?;
    read_option_cross_margin_params_at(ctx, state, contract, base)
}

pub fn get_market_fee_by_category(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
    market: Felt,
    category: FeeCategory,
    is_maker: bool,
) -> Result<i128, ExecutionError> {
    if matches!(category, FeeCategory::Unspecified) {
        return Ok(0);
    }

    let per_market = read_market_fee_config(ctx, state, contract, market)?;
    let config =
        if per_market.exists { per_market } else { read_global_market_fee_config(ctx, state, contract, kind)? };
    felt_to_i128(match (category, is_maker) {
        (FeeCategory::API, true) => config.maker_api,
        (FeeCategory::API, false) => config.taker_api,
        (FeeCategory::RPI, true) => config.maker_rpi,
        (FeeCategory::RPI, false) => config.taker_rpi,
        (FeeCategory::Interactive, true) => config.maker_interactive,
        (FeeCategory::Interactive, false) => config.taker_interactive,
        (FeeCategory::Unspecified, _) => Felt::ZERO,
    })
}

pub fn get_base_asset_option_fee_by_category(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
    base_asset: Felt,
    category: FeeCategory,
    is_maker: bool,
) -> Result<FeeWithCap, ExecutionError> {
    if matches!(category, FeeCategory::Unspecified) {
        return Ok(FeeWithCap { fee: Felt::ZERO, fee_cap: Felt::ZERO, fee_floor: Felt::ZERO });
    }

    let per_base = read_base_asset_option_fee_config(ctx, state, contract, kind, base_asset)?;
    if fee_with_cap_exists(&per_base.exists) {
        return Ok(select_option_fee_slot(&per_base, category, is_maker));
    }

    let global = read_global_option_fee_config(ctx, state, contract, kind)?;
    Ok(select_option_fee_slot(&global, category, is_maker))
}

pub fn get_market_fee_provision_rate(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
) -> Result<i128, ExecutionError> {
    let config = read_global_market_fee_config(ctx, state, contract, kind)?;
    felt_to_i128(config.max_fee_rate)
}

pub fn get_option_fee_provision_rate(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
) -> Result<FeeWithCap, ExecutionError> {
    let config = read_global_option_fee_config(ctx, state, contract, kind)?;
    Ok(config.max_fee_rate)
}

pub fn get_settlement_fee_config(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
) -> Result<FeeWithCap, ExecutionError> {
    let base = read_component_map_base(
        ctx,
        state,
        contract,
        *assets_manager_layout::FEE_BASE,
        "settlement_fee_config",
        "fee.settlement_fee_config",
        *SETTLEMENT_FEE_CONFIG_VAR,
        *SETTLEMENT_FEE_CONFIG_DOTTED_VAR,
        kind,
    )?;
    read_fee_with_cap_at(ctx, state, contract, base)
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

fn get_option_kind(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<Option<Felt>, ExecutionError> {
    let asset = read_option_asset_with_fallback(ctx, state, contract, market)?;
    if asset.market != market {
        return Ok(None);
    }
    if asset.expiry_time == Felt::ZERO {
        Ok(Some(asset_kind_perp_option()))
    } else {
        Ok(Some(asset_kind_dated_option()))
    }
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

fn read_spot_asset_with_fallback(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<SpotAsset, ExecutionError> {
    let base = read_component_map_base(
        ctx,
        state,
        contract,
        *assets_manager_layout::SPOT_BASE,
        "spot_asset",
        "spot.spot_asset",
        *SPOT_ASSET_VAR,
        *SPOT_ASSET_DOTTED_VAR,
        market,
    )?;
    let market_felt = ctx.storage_read(state, contract, base)?;
    let base_token_address = ContractAddress(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?);
    let base_token_name = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let quote_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
    Ok(SpotAsset { market: market_felt, base_token_address, base_token_name, quote_asset })
}

fn read_perpetual_asset_with_fallback(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<PerpetualAsset, ExecutionError> {
    let base = read_component_map_base(
        ctx,
        state,
        contract,
        *assets_manager_layout::FUTURE_BASE,
        "perpetual_future_asset",
        "future.perpetual_future_asset",
        *FUTURE_ASSET_VAR,
        *FUTURE_ASSET_DOTTED_VAR,
        market,
    )?;
    let market_felt = ctx.storage_read(state, contract, base)?;
    let base_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    let quote_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let tick_size = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
    let imf_base = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    let imf_factor = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
    let mmf_factor = ctx.storage_read(state, contract, storage_key_with_offset(base, 6))?;
    let imf_shift = ctx.storage_read(state, contract, storage_key_with_offset(base, 7))?;
    Ok(PerpetualAsset {
        market: market_felt,
        base_asset,
        quote_asset,
        tick_size,
        margin_params: PerpetualMarginParams { imf_base, imf_factor, mmf_factor, imf_shift },
    })
}

fn read_option_asset_with_fallback(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<OptionAsset, ExecutionError> {
    let base = read_component_map_base(
        ctx,
        state,
        contract,
        *assets_manager_layout::OPTION_BASE,
        "option_asset",
        "option.option_asset",
        *OPTION_ASSET_VAR,
        *OPTION_ASSET_DOTTED_VAR,
        market,
    )?;
    let market_felt = ctx.storage_read(state, contract, base)?;
    let base_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    let quote_asset = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let tick_size = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
    let option_type = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    let strike = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
    let expiry_time = ctx.storage_read(state, contract, storage_key_with_offset(base, 6))?;
    Ok(OptionAsset { market: market_felt, base_asset, quote_asset, tick_size, option_type, strike, expiry_time })
}

#[allow(clippy::too_many_arguments)]
fn read_component_map_base(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    component_base: StorageKey,
    plain_name: &str,
    dotted_name: &str,
    plain_var: Felt,
    dotted_var: Felt,
    key: Felt,
) -> Result<StorageKey, ExecutionError> {
    let candidates = [
        storage_key_for_map_poseidon_with_base_named(plain_var, key, plain_name),
        storage_key_for_map_poseidon_with_base_named(dotted_var, key, dotted_name),
        storage_key_for_substorage_map_poseidon_with_var_named(component_base, plain_var, key, plain_name),
        storage_key_for_substorage_map_poseidon_with_var_named(component_base, dotted_var, key, dotted_name),
        storage_key_for_substorage_map_poseidon_add_with_var_named(component_base, plain_var, key, plain_name),
        storage_key_for_substorage_map_poseidon_add_with_var_named(component_base, dotted_var, key, dotted_name),
        storage_key_for_substorage_map_poseidon_hash_with_var_named(component_base, plain_var, key, plain_name),
        storage_key_for_substorage_map_poseidon_hash_with_var_named(component_base, dotted_var, key, dotted_name),
    ];
    for base in candidates {
        let first = ctx.storage_read(state, contract, base)?;
        if first != Felt::ZERO {
            return Ok(base);
        }
    }
    Ok(candidates[2])
}

#[allow(dead_code)]
fn read_component_map_base_with_fallback(
    component_base: StorageKey,
    plain_name: &str,
    dotted_name: &str,
    key: Felt,
) -> [StorageKey; 8] {
    [
        storage_key_for_map_poseidon(plain_name, key),
        storage_key_for_map_poseidon(dotted_name, key),
        storage_key_for_substorage_map_poseidon(component_base, plain_name, key),
        storage_key_for_substorage_map_poseidon(component_base, dotted_name, key),
        storage_key_for_substorage_map_poseidon_add(component_base, plain_name, key),
        storage_key_for_substorage_map_poseidon_add(component_base, dotted_name, key),
        storage_key_for_substorage_map_poseidon_hash(component_base, plain_name, key),
        storage_key_for_substorage_map_poseidon_hash(component_base, dotted_name, key),
    ]
}

fn read_fee_component_var_base(name: &str) -> [StorageKey; 2] {
    [
        storage_key_for_substorage_var_poseidon(*assets_manager_layout::FEE_BASE, name),
        storage_key_for_substorage_var_add(*assets_manager_layout::FEE_BASE, name),
    ]
}

fn read_market_fee_config(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    market: Felt,
) -> Result<MarketFeeConfig, ExecutionError> {
    let candidates = [
        storage_key_for_map_poseidon_with_base_named(*MARKET_FEE_CONFIG_VAR, market, "market_fee_config"),
        storage_key_for_map_poseidon_with_base_named(*MARKET_FEE_CONFIG_DOTTED_VAR, market, "fee.market_fee_config"),
        storage_key_for_substorage_map_poseidon_with_var_named(
            *assets_manager_layout::FEE_BASE,
            *MARKET_FEE_CONFIG_VAR,
            market,
            "market_fee_config",
        ),
        storage_key_for_substorage_map_poseidon_with_var_named(
            *assets_manager_layout::FEE_BASE,
            *MARKET_FEE_CONFIG_DOTTED_VAR,
            market,
            "fee.market_fee_config",
        ),
    ];
    for base in candidates {
        let exists = ctx.storage_read(state, contract, base)?;
        if exists != Felt::ZERO {
            return read_market_fee_config_at(ctx, state, contract, base);
        }
    }
    read_market_fee_config_at(ctx, state, contract, candidates[2])
}

fn read_global_market_fee_config(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
) -> Result<MarketFeeConfig, ExecutionError> {
    let name = if kind == asset_kind_spot() { "global_spot_fee_config" } else { "global_future_fee_config" };
    for base in read_fee_component_var_base(name) {
        let config = read_market_fee_config_at(ctx, state, contract, base)?;
        if config.exists || any_market_fee_value(&config) {
            return Ok(config);
        }
    }
    read_market_fee_config_at(
        ctx,
        state,
        contract,
        storage_key_for_substorage_var_poseidon(*assets_manager_layout::FEE_BASE, name),
    )
}

fn read_global_option_fee_config(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
) -> Result<OptionFeeConfigLite, ExecutionError> {
    let name = if kind == asset_kind_dated_option() {
        "global_dated_option_fee_config"
    } else {
        "global_perpetual_option_fee_config"
    };
    for base in read_fee_component_var_base(name) {
        let config = read_option_fee_config_at(ctx, state, contract, base)?;
        if fee_with_cap_exists(&config.exists) || any_option_fee_value(&config) {
            return Ok(config);
        }
    }
    read_option_fee_config_at(
        ctx,
        state,
        contract,
        storage_key_for_substorage_var_poseidon(*assets_manager_layout::FEE_BASE, name),
    )
}

fn read_base_asset_option_fee_config(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    kind: Felt,
    base_asset: Felt,
) -> Result<OptionFeeConfigLite, ExecutionError> {
    let (plain_name, dotted_name, plain_var, dotted_var) = if kind == asset_kind_dated_option() {
        (
            "base_asset_dated_option_fee_config",
            "fee.base_asset_dated_option_fee_config",
            *BASE_ASSET_DATED_OPTION_FEE_CONFIG_VAR,
            *BASE_ASSET_DATED_OPTION_FEE_CONFIG_DOTTED_VAR,
        )
    } else {
        (
            "base_asset_perpetual_option_fee_config",
            "fee.base_asset_perpetual_option_fee_config",
            *BASE_ASSET_PERP_OPTION_FEE_CONFIG_VAR,
            *BASE_ASSET_PERP_OPTION_FEE_CONFIG_DOTTED_VAR,
        )
    };
    let base = read_component_map_base(
        ctx,
        state,
        contract,
        *assets_manager_layout::FEE_BASE,
        plain_name,
        dotted_name,
        plain_var,
        dotted_var,
        base_asset,
    )?;
    read_option_fee_config_at(ctx, state, contract, base)
}

fn read_market_fee_config_at(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    base: StorageKey,
) -> Result<MarketFeeConfig, ExecutionError> {
    Ok(MarketFeeConfig {
        exists: ctx.storage_read(state, contract, base)? != Felt::ZERO,
        maker_api: ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?,
        taker_api: ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?,
        maker_rpi: ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?,
        taker_rpi: ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?,
        maker_interactive: ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?,
        taker_interactive: ctx.storage_read(state, contract, storage_key_with_offset(base, 6))?,
        max_fee_rate: ctx.storage_read(state, contract, storage_key_with_offset(base, 7))?,
    })
}

fn read_option_fee_config_at(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    base: StorageKey,
) -> Result<OptionFeeConfigLite, ExecutionError> {
    Ok(OptionFeeConfigLite {
        exists: ctx.storage_read(state, contract, base)?,
        maker_api: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 1))?,
        taker_api: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 4))?,
        maker_rpi: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 7))?,
        taker_rpi: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 10))?,
        maker_interactive: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 13))?,
        taker_interactive: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 16))?,
        max_fee_rate: read_fee_with_cap_at(ctx, state, contract, storage_key_with_offset(base, 19))?,
    })
}

fn read_option_cross_margin_params_at(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    base: StorageKey,
) -> Result<OptionCrossMarginParams, ExecutionError> {
    Ok(OptionCrossMarginParams {
        imf: read_option_margin_params_at(ctx, state, contract, base)?,
        mmf: read_option_margin_params_at(ctx, state, contract, storage_key_with_offset(base, 5))?,
    })
}

fn read_option_margin_params_at(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    base: StorageKey,
) -> Result<OptionMarginParams, ExecutionError> {
    Ok(OptionMarginParams {
        premium_multiplier: ctx.storage_read(state, contract, base)?,
        long_itm: ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?,
        short_itm: ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?,
        short_otm: ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?,
        short_put_cap: ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?,
    })
}

fn read_fee_with_cap_at(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    base: StorageKey,
) -> Result<FeeWithCap, ExecutionError> {
    Ok(FeeWithCap {
        fee: ctx.storage_read(state, contract, base)?,
        fee_cap: ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?,
        fee_floor: ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?,
    })
}

fn select_option_fee_slot(config: &OptionFeeConfigLite, category: FeeCategory, is_maker: bool) -> FeeWithCap {
    match (category, is_maker) {
        (FeeCategory::API, true) => config.maker_api.clone(),
        (FeeCategory::API, false) => config.taker_api.clone(),
        (FeeCategory::RPI, true) => config.maker_rpi.clone(),
        (FeeCategory::RPI, false) => config.taker_rpi.clone(),
        (FeeCategory::Interactive, true) => config.maker_interactive.clone(),
        (FeeCategory::Interactive, false) => config.taker_interactive.clone(),
        (FeeCategory::Unspecified, _) => FeeWithCap { fee: Felt::ZERO, fee_cap: Felt::ZERO, fee_floor: Felt::ZERO },
    }
}

fn fee_with_cap_exists(value: &Felt) -> bool {
    *value != Felt::ZERO
}

fn any_market_fee_value(config: &MarketFeeConfig) -> bool {
    config.maker_api != Felt::ZERO
        || config.taker_api != Felt::ZERO
        || config.maker_rpi != Felt::ZERO
        || config.taker_rpi != Felt::ZERO
        || config.maker_interactive != Felt::ZERO
        || config.taker_interactive != Felt::ZERO
        || config.max_fee_rate != Felt::ZERO
}

fn any_option_fee_value(config: &OptionFeeConfigLite) -> bool {
    config.maker_api.fee != Felt::ZERO
        || config.taker_api.fee != Felt::ZERO
        || config.maker_rpi.fee != Felt::ZERO
        || config.taker_rpi.fee != Felt::ZERO
        || config.maker_interactive.fee != Felt::ZERO
        || config.taker_interactive.fee != Felt::ZERO
        || config.max_fee_rate.fee != Felt::ZERO
}

fn felt_to_i128(value: Felt) -> Result<i128, ExecutionError> {
    let bytes = value.to_bytes_be();
    let is_negative = bytes[0] & 0x80 != 0;
    if !is_negative {
        if bytes[..16].iter().any(|b| *b != 0) {
            return Err(ExecutionError::ExecutionFailed("value too large for i128".to_string()));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&bytes[16..]);
        return Ok(i128::from_be_bytes(arr));
    }

    let neg = Felt::ZERO - value;
    let neg_bytes = neg.to_bytes_be();
    if neg_bytes[..16].iter().any(|b| *b != 0) {
        return Err(ExecutionError::ExecutionFailed("negative value too small for i128".to_string()));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&neg_bytes[16..]);
    Ok(-i128::from_be_bytes(arr))
}

#[derive(Clone, Debug)]
struct OptionFeeConfigLite {
    exists: Felt,
    maker_api: FeeWithCap,
    taker_api: FeeWithCap,
    maker_rpi: FeeWithCap,
    taker_rpi: FeeWithCap,
    maker_interactive: FeeWithCap,
    taker_interactive: FeeWithCap,
    max_fee_rate: FeeWithCap,
}
