//! Paraclear contract (settle_trade_v3) - incremental implementation.
#![allow(clippy::too_many_arguments)]

mod names;
mod selectors;

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::contracts::paradex::schema::account_component_layout as account_layout;
use crate::contracts::paradex::schema::paraclear_layout;
use crate::contracts::paradex::schema::paraclear_layout as layout;
use crate::contracts::paradex::schema::token_component_layout as token_layout;
use crate::contracts::paradex::{assets_manager, oracle};
use crate::contracts::ExecutionError;
use crate::core::context::ExecutionContext;
use crate::core::state::StateReader;
use crate::core::storage::{
    event_selector, pedersen_hash, sn_keccak, storage_key_for_map, storage_key_for_map2, storage_key_for_map2_poseidon,
    storage_key_for_map2_with_base_named, storage_key_for_map_poseidon, storage_key_for_map_with_base_named,
    storage_key_for_substorage_map2_poseidon, storage_key_for_substorage_map2_poseidon_add,
    storage_key_for_substorage_map2_poseidon_hash, storage_key_for_substorage_map_poseidon,
    storage_key_for_substorage_map_poseidon_add, storage_key_for_substorage_map_poseidon_hash,
    storage_key_for_substorage_var_add, storage_key_for_substorage_var_poseidon, storage_key_for_variable,
    storage_key_with_offset,
};
use crate::core::types::{ContractAddress, ExecutionResult, StorageKey};

use crate::contracts::paradex::schema::paraclear_types::{FeeWithCapRequest, OrderCategory, OrderV3, TradeRequestV3};
pub(crate) use names::PRECOMPUTED_NAMES;
pub(crate) use selectors::FUNCTION_NAMES;
use selectors::{get_function_name as selector_function_name, settle_trade_v3_selector};

/// Name of the contract (for debugging/logging).
pub const NAME: &str = "Paraclear";

/// Supported Paradex Paraclear class hash.
pub const CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x05e9bdfbd0b2b461a42052f43a38663b1d53f7ce8a9537bdc06b857b7508a13a");

pub fn supports_class_hash(class_hash: Felt) -> bool {
    class_hash == CLASS_HASH
}

// Precomputed storage map bases (sn_keccak) for hot paths.
static PERP_BALANCE_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("Paraclear_perpetual_asset_balance".as_bytes()));
static TOKEN_BALANCE_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("Paraclear_token_asset_balance".as_bytes()));
static PERP_BALANCE_TAIL_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak("Paraclear_perpetual_asset_balance_tail".as_bytes()));
static TOKEN_BALANCE_TAIL_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("Paraclear_token_asset_balance_tail".as_bytes()));
static PERP_ASSET_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("Paraclear_perpetual_asset".as_bytes()));
static PERP_ASSET_MIN_SIZE_INCREMENT_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak("Paraclear_perpetual_asset_min_size_increment".as_bytes()));
static PERP_TOTAL_REALIZED_PNL_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak("Paraclear_total_perpetual_asset_realized_pnl".as_bytes()));
static PERP_TOTAL_REALIZED_FUNDING_PNL_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak("Paraclear_total_perpetual_asset_realized_funding_pnl".as_bytes()));
static INVARIANTS_PERP_ASSET_INFO_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak("invariants_perpetual_asset_info".as_bytes()));
static PERP_MARKET_FEE_CONFIG_V2_BASE: Lazy<Felt> =
    Lazy::new(|| sn_keccak("perpetual_future_market_fee_config_v2".as_bytes()));
static PERP_MAX_FEE_RATE_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak("perpetual_future_max_fee_rate".as_bytes()));

#[derive(Clone, Copy, Debug)]
enum InnerTimingField {
    LoadAccountStateForRisk,
    IsRiskyAfterTradeLoaded,
    EnforceMaxAssetsPerAccount,
    SettlePerpetual,
    SettleSpot,
    LoadTokenBalances,
    LoadPerpBalancesAndPrices,
    CountTokenAssets,
    CountPerpAssets,
    MarginRequirementAndTotalUpnl,
    GetAssetValue,
}

#[derive(Default, Debug)]
struct InnerTiming {
    load_account_state_for_risk: Duration,
    is_risky_after_trade_loaded: Duration,
    enforce_max_assets_per_account: Duration,
    settle_perpetual: Duration,
    settle_spot: Duration,
    load_token_balances: Duration,
    load_perp_balances_and_prices: Duration,
    count_token_assets: Duration,
    count_perp_assets: Duration,
    margin_requirement_and_total_upnl: Duration,
    get_asset_value: Duration,
    trade_id: Option<Felt>,
    market: Option<Felt>,
    maker: Option<Felt>,
    taker: Option<Felt>,
}

impl InnerTiming {
    fn add(&mut self, field: InnerTimingField, duration: Duration) {
        match field {
            InnerTimingField::LoadAccountStateForRisk => self.load_account_state_for_risk += duration,
            InnerTimingField::IsRiskyAfterTradeLoaded => self.is_risky_after_trade_loaded += duration,
            InnerTimingField::EnforceMaxAssetsPerAccount => self.enforce_max_assets_per_account += duration,
            InnerTimingField::SettlePerpetual => self.settle_perpetual += duration,
            InnerTimingField::SettleSpot => self.settle_spot += duration,
            InnerTimingField::LoadTokenBalances => self.load_token_balances += duration,
            InnerTimingField::LoadPerpBalancesAndPrices => self.load_perp_balances_and_prices += duration,
            InnerTimingField::CountTokenAssets => self.count_token_assets += duration,
            InnerTimingField::CountPerpAssets => self.count_perp_assets += duration,
            InnerTimingField::MarginRequirementAndTotalUpnl => self.margin_requirement_and_total_upnl += duration,
            InnerTimingField::GetAssetValue => self.get_asset_value += duration,
        }
    }

    fn set_trade_context(&mut self, trade: &TradeRequestV3) {
        self.trade_id = Some(trade.id);
        self.market = Some(trade.maker_order.market);
        self.maker = Some(trade.maker_order.account.0);
        self.taker = Some(trade.taker_order.account.0);
    }

    fn log(&self) {
        if !crate::config::inner_timing_log_enabled() {
            return;
        }

        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let trade_id = self.trade_id.unwrap_or(Felt::ZERO);
        let market = self.market.unwrap_or(Felt::ZERO);
        let maker = self.maker.unwrap_or(Felt::ZERO);
        let taker = self.taker.unwrap_or(Felt::ZERO);
        tracing::info!(
            "inner_timings load_account_state_for_risk_ms={:.3}, \
             is_risky_after_trade_loaded={:.3}, enforce_max_assets_per_account={:.3}, \
             settle_perpetual={:.3}, settle_spot={:.3}, load_token_balances={:.3}, \
             load_perp_balances_and_prices={:.3}, count_token_assets={:.3}, \
             count_perp_assets={:.3}, margin_requirement_and_total_upnl={:.3}, \
             get_asset_value={:.3} (trade_id={:#x}, market={:#x}, maker={:#x}, taker={:#x})",
            ms(self.load_account_state_for_risk),
            ms(self.is_risky_after_trade_loaded),
            ms(self.enforce_max_assets_per_account),
            ms(self.settle_perpetual),
            ms(self.settle_spot),
            ms(self.load_token_balances),
            ms(self.load_perp_balances_and_prices),
            ms(self.count_token_assets),
            ms(self.count_perp_assets),
            ms(self.margin_requirement_and_total_upnl),
            ms(self.get_asset_value),
            trade_id,
            market,
            maker,
            taker,
        );
    }
}

thread_local! {
    static INNER_TIMING: RefCell<Option<InnerTiming>> = const { RefCell::new(None) };
}

struct InnerTimingGuard {
    enabled: bool,
}

impl InnerTimingGuard {
    fn new() -> Self {
        if crate::config::execution_log_inner_enabled() {
            INNER_TIMING.with(|slot| {
                *slot.borrow_mut() = Some(InnerTiming::default());
            });
            Self { enabled: true }
        } else {
            Self { enabled: false }
        }
    }
}

impl Drop for InnerTimingGuard {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let timing = INNER_TIMING.with(|slot| slot.borrow_mut().take());
        if let Some(timing) = timing {
            timing.log();
        }
    }
}

fn set_inner_timing_context(trade: &TradeRequestV3) {
    if !crate::config::execution_log_inner_enabled() {
        return;
    }
    INNER_TIMING.with(|slot| {
        if let Some(ref mut timing) = *slot.borrow_mut() {
            timing.set_trade_context(trade);
        }
    });
}

fn time_inner<R, F: FnOnce() -> R>(field: InnerTimingField, f: F) -> R {
    if !crate::config::execution_log_inner_enabled() {
        return f();
    }
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed();
    INNER_TIMING.with(|slot| {
        if let Some(ref mut timing) = *slot.borrow_mut() {
            timing.add(field, elapsed);
        }
    });
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum PerpStorageMode {
    LegacyPedersen,
    LegacyPoseidon,
    Substorage,
    SubstorageAdd,
    SubstorageHash,
}

fn perp_storage_base() -> StorageKey {
    *paraclear_layout::PERPETUAL_FUTURE_BASE
}

fn resolve_perp_storage_mode<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
) -> Result<PerpStorageMode, ExecutionError> {
    let key = storage_key_for_map_with_base_named(*PERP_ASSET_BASE, market, "Paraclear_perpetual_asset");
    let stored_market = ctx.storage_read(state, contract, key)?;
    if stored_market == market {
        return Ok(PerpStorageMode::LegacyPedersen);
    }
    Ok(PerpStorageMode::LegacyPedersen)
}

fn perp_map_key(mode: PerpStorageMode, name: &str, key: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map(name, key),
        PerpStorageMode::LegacyPoseidon => storage_key_for_map_poseidon(name, key),
        PerpStorageMode::Substorage => storage_key_for_substorage_map_poseidon(perp_storage_base(), name, key),
        PerpStorageMode::SubstorageAdd => storage_key_for_substorage_map_poseidon_add(perp_storage_base(), name, key),
        PerpStorageMode::SubstorageHash => storage_key_for_substorage_map_poseidon_hash(perp_storage_base(), name, key),
    }
}

fn perp_map2_key(mode: PerpStorageMode, name: &str, key1: Felt, key2: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map2(name, key1, key2),
        PerpStorageMode::LegacyPoseidon => storage_key_for_map2_poseidon(name, key1, key2),
        PerpStorageMode::Substorage => storage_key_for_substorage_map2_poseidon(perp_storage_base(), name, key1, key2),
        PerpStorageMode::SubstorageAdd => {
            storage_key_for_substorage_map2_poseidon_add(perp_storage_base(), name, key1, key2)
        }
        PerpStorageMode::SubstorageHash => {
            storage_key_for_substorage_map2_poseidon_hash(perp_storage_base(), name, key1, key2)
        }
    }
}

fn perp_var_key(mode: PerpStorageMode, name: &str) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen | PerpStorageMode::LegacyPoseidon => storage_key_for_variable(name),
        PerpStorageMode::Substorage | PerpStorageMode::SubstorageHash => {
            storage_key_for_substorage_var_poseidon(perp_storage_base(), name)
        }
        PerpStorageMode::SubstorageAdd => storage_key_for_substorage_var_add(perp_storage_base(), name),
    }
}

fn resolve_perp_storage_mode_for_account<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<PerpStorageMode, ExecutionError> {
    let tail_key = perp_balance_tail_key(PerpStorageMode::LegacyPedersen, account);
    let _tail = ctx.storage_read(state, contract, tail_key)?;
    Ok(PerpStorageMode::LegacyPedersen)
}

fn perp_balance_tail_key(mode: PerpStorageMode, account: ContractAddress) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map_with_base_named(
            *PERP_BALANCE_TAIL_BASE,
            account.0,
            "Paraclear_perpetual_asset_balance_tail",
        ),
        _ => perp_map_key(mode, "Paraclear_perpetual_asset_balance_tail", account.0),
    }
}

#[cfg(test)]
fn mode_to_u8(mode: PerpStorageMode) -> u8 {
    match mode {
        PerpStorageMode::LegacyPedersen => 0,
        PerpStorageMode::LegacyPoseidon => 1,
        PerpStorageMode::Substorage => 2,
        PerpStorageMode::SubstorageAdd => 3,
        PerpStorageMode::SubstorageHash => 4,
    }
}

fn resolve_perp_balance_base<S: StateReader>(
    _state: &S,
    _ctx: &mut ExecutionContext,
    _contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
) -> Result<StorageKey, ExecutionError> {
    let inner = perp_balance_inner_base(account);
    Ok(perp_balance_base_from_inner(inner, market))
}

fn resolve_token_balance_base<S: StateReader>(
    _state: &S,
    _ctx: &mut ExecutionContext,
    _contract: ContractAddress,
    account: ContractAddress,
    token: ContractAddress,
) -> Result<StorageKey, ExecutionError> {
    let inner = token_balance_inner_base(account);
    Ok(token_balance_base_from_inner(inner, token))
}

fn perp_balance_inner_base(account: ContractAddress) -> Felt {
    pedersen_hash(&PERP_BALANCE_BASE, &account.0)
}

fn perp_balance_base_from_inner(inner: Felt, market: Felt) -> StorageKey {
    StorageKey(pedersen_hash(&inner, &market))
}

fn token_balance_inner_base(account: ContractAddress) -> Felt {
    pedersen_hash(&TOKEN_BALANCE_BASE, &account.0)
}

fn token_balance_base_from_inner(inner: Felt, token: ContractAddress) -> StorageKey {
    StorageKey(pedersen_hash(&inner, &token.0))
}

fn read_perp_price_i128<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    oracle_address: ContractAddress,
    market: Felt,
) -> Result<i128, ExecutionError> {
    let tick = oracle::read_tick_data(ctx, state, oracle_address, market)?;
    let value = felt_to_i128(tick.asset_value)?;
    Ok(value)
}

fn read_funding_index_i128<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    oracle_address: ContractAddress,
    market: Felt,
) -> Result<i128, ExecutionError> {
    let idx = oracle::read_funding_index(ctx, state, oracle_address, market)?;
    let value = felt_to_i128(idx)?;
    Ok(value)
}

fn resolve_token_balance_tail_key<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<StorageKey, ExecutionError> {
    let key =
        storage_key_for_map_with_base_named(*TOKEN_BALANCE_TAIL_BASE, account.0, "Paraclear_token_asset_balance_tail");
    let _ = state;
    let _ = contract;
    Ok(key)
}

fn perp_asset_key(mode: PerpStorageMode, market: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => {
            storage_key_for_map_with_base_named(*PERP_ASSET_BASE, market, "Paraclear_perpetual_asset")
        }
        _ => perp_map_key(mode, "Paraclear_perpetual_asset", market),
    }
}

#[allow(dead_code)]
fn perp_asset_min_size_increment_key(mode: PerpStorageMode, market: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map_with_base_named(
            *PERP_ASSET_MIN_SIZE_INCREMENT_BASE,
            market,
            "Paraclear_perpetual_asset_min_size_increment",
        ),
        _ => perp_map_key(mode, "Paraclear_perpetual_asset_min_size_increment", market),
    }
}

#[allow(dead_code)]
fn perp_total_realized_pnl_key(mode: PerpStorageMode, market: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map_with_base_named(
            *PERP_TOTAL_REALIZED_PNL_BASE,
            market,
            "Paraclear_total_perpetual_asset_realized_pnl",
        ),
        _ => perp_map_key(mode, "Paraclear_total_perpetual_asset_realized_pnl", market),
    }
}

#[allow(dead_code)]
fn perp_total_realized_funding_pnl_key(mode: PerpStorageMode, market: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map_with_base_named(
            *PERP_TOTAL_REALIZED_FUNDING_PNL_BASE,
            market,
            "Paraclear_total_perpetual_asset_realized_funding_pnl",
        ),
        _ => perp_map_key(mode, "Paraclear_total_perpetual_asset_realized_funding_pnl", market),
    }
}

fn perp_invariants_info_key(mode: PerpStorageMode, market: Felt, account: ContractAddress) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map2_with_base_named(
            *INVARIANTS_PERP_ASSET_INFO_BASE,
            market,
            account.0,
            "invariants_perpetual_asset_info",
        ),
        _ => perp_map2_key(mode, "invariants_perpetual_asset_info", market, account.0),
    }
}

fn perp_market_fee_config_v2_key(mode: PerpStorageMode, market: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => storage_key_for_map_with_base_named(
            *PERP_MARKET_FEE_CONFIG_V2_BASE,
            market,
            "perpetual_future_market_fee_config_v2",
        ),
        _ => perp_map_key(mode, "perpetual_future_market_fee_config_v2", market),
    }
}

fn perp_max_fee_rate_key(mode: PerpStorageMode, market: Felt) -> StorageKey {
    match mode {
        PerpStorageMode::LegacyPedersen => {
            storage_key_for_map_with_base_named(*PERP_MAX_FEE_RATE_BASE, market, "perpetual_future_max_fee_rate")
        }
        _ => perp_map_key(mode, "perpetual_future_max_fee_rate", market),
    }
}

fn perp_mmf_factor_key(mode: PerpStorageMode) -> StorageKey {
    perp_var_key(mode, "perpetual_futures_mmf_factor")
}

pub fn supports_selector(selector: Felt) -> bool {
    selector == settle_trade_v3_selector()
}

pub fn get_function_name(selector: Felt) -> Option<String> {
    selector_function_name(selector).map(str::to_string)
}

pub fn execute<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
) -> Result<ExecutionResult, ExecutionError> {
    execute_with_timestamp(state, contract, selector, calldata, caller, 0)
}

pub fn execute_with_timestamp<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    selector: Felt,
    calldata: &[Felt],
    caller: ContractAddress,
    block_timestamp: u64,
) -> Result<ExecutionResult, ExecutionError> {
    if selector != settle_trade_v3_selector() {
        return Err(ExecutionError::UnknownSelector(selector));
    }

    let _inner_timing_guard = InnerTimingGuard::new();
    let (trade, _remaining) = decode_trade_request_v3(calldata)?;
    set_inner_timing_context(&trade);

    let mut ctx = ExecutionContext::with_timestamp(block_timestamp);

    if let Err(reason) = validate_trade_basic(&trade) {
        ctx.fail(reason.to_string());
        return Ok(ctx.build_result());
    }

    if block_timestamp != 0 {
        let traded_at = felt_to_u64(trade.traded_at)?;
        let max_allowed = block_timestamp.saturating_add(3600);
        if traded_at > max_allowed {
            ctx.fail("Trade: traded_at from future".to_string());
            return Ok(ctx.build_result());
        }
    }

    // Resolve dependencies.
    let assets_manager_address = read_contract_address(&mut ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;
    let oracle_address =
        read_contract_address(&mut ctx, state, contract, *layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE)?;

    // log_perp_balance_candidates(trade.maker_order.account, trade.maker_order.market);
    // log_perp_balance_candidates(trade.taker_order.account, trade.maker_order.market);

    let asset_kind = get_asset_kind(&mut ctx, state, contract, assets_manager_address, trade.maker_order.market)?;
    if asset_kind == assets_manager::ASSET_KIND_UNSUPPORTED {
        emit_settle_trade_failed(
            &mut ctx,
            ErrorCodes::trade_market_not_supported(),
            Errors::trade_market_not_supported(),
            &trade,
        );
        ctx.set_retdata(vec![Felt::ZERO]);
        return Ok(ctx.build_result());
    }

    if asset_kind != assets_manager::asset_kind_spot() {
        let mode = resolve_perp_storage_mode(&mut ctx, state, contract, trade.maker_order.market)?;
        ensure_perpetual_balance(&mut ctx, state, contract, trade.maker_order.account, trade.maker_order.market, mode)?;
        ensure_perpetual_balance(&mut ctx, state, contract, trade.taker_order.account, trade.maker_order.market, mode)?;
    }

    enforce_max_assets_per_account(&mut ctx, state, contract, &trade)?;

    if !trade_support_fee_v2(&trade) {
        ctx.fail("Trade must support fee v2".to_string());
        return Ok(ctx.build_result());
    }

    // Fetch settlement token price (needed by both spot/perp flows).
    let settlement_token_price =
        oracle::read_settlement_token_price(&mut ctx, state, oracle_address, oracle_settlement_key())?;
    if settlement_token_price == Felt::ZERO {
        return Err(ExecutionError::ExecutionFailed("settlement_token_price is zero".to_string()));
    }

    // Dispatch by asset kind.
    if asset_kind == assets_manager::asset_kind_spot() {
        settle_spot(state, contract, &trade, settlement_token_price, caller, &mut ctx)?;
    } else {
        settle_perpetual(
            state,
            contract,
            &trade,
            settlement_token_price,
            oracle_address,
            asset_kind,
            caller,
            &mut ctx,
        )?;
    }

    if !ctx.is_failed() {
        ctx.set_retdata(vec![Felt::ONE]);
    }
    Ok(ctx.build_result())
}

fn settle_spot<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
    settlement_token_price: Felt,
    caller: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    time_inner(InnerTimingField::SettleSpot, || {
        let assets_manager_address = read_contract_address(ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;
        let spot_asset =
            assets_manager::get_base_token_asset(ctx, state, assets_manager_address, trade.maker_order.market)?;
        let settlement_token_address = read_settlement_token_address(ctx, state, contract)?;
        let maker_state =
            load_account_state_for_risk(ctx, state, contract, trade.maker_order.account, settlement_token_address)?;
        let taker_state =
            load_account_state_for_risk(ctx, state, contract, trade.taker_order.account, settlement_token_address)?;

        let size_i128 = felt_to_i128(trade.size)?;
        let price_i128 = felt_to_i128(trade.price)?;
        let settlement_token_price_i128 = felt_to_i128(settlement_token_price)?;
        let quote_amount = mul_128_scaled(size_i128, price_i128)?;
        let settlement_amount = div_128_scaled(quote_amount, settlement_token_price_i128)?;

        let (taker_base_delta, taker_quote_delta, maker_base_delta, maker_quote_delta) =
            if trade.taker_order.side == buy_side() {
                (size_i128, -settlement_amount, -size_i128, settlement_amount)
            } else {
                (-size_i128, settlement_amount, size_i128, -settlement_amount)
            };

        let maker_base_before = token_amount_from_state(&maker_state, spot_asset.token_address);
        let taker_base_before = token_amount_from_state(&taker_state, spot_asset.token_address);

        if trade.maker_order.side == sell_side() && maker_base_before < size_i128 {
            emit_settle_trade_failed(
                ctx,
                ErrorCodes::trade_maker_not_enough_balance(),
                Errors::trade_maker_not_enough_balance(),
                trade,
            );
            ctx.set_retdata(vec![Felt::ZERO]);
            ctx.fail("maker not enough balance".to_string());
            return Ok(());
        }

        if trade.taker_order.side == sell_side() && taker_base_before < size_i128 {
            emit_settle_trade_failed(
                ctx,
                ErrorCodes::trade_taker_not_enough_balance(),
                Errors::trade_taker_not_enough_balance(),
                trade,
            );
            ctx.set_retdata(vec![Felt::ZERO]);
            ctx.fail("taker not enough balance".to_string());
            return Ok(());
        }

        let maker_settlement_before = token_amount_from_state(&maker_state, settlement_token_address);
        let taker_settlement_before = token_amount_from_state(&taker_state, settlement_token_address);

        let maker_updated_settlement = maker_settlement_before + maker_quote_delta;
        let taker_updated_settlement = taker_settlement_before + taker_quote_delta;

        let maker_fee = base_fee_spot(ctx, state, contract, trade, true, &maker_state.fee_rates)?;
        let taker_fee = base_fee_spot(ctx, state, contract, trade, false, &taker_state.fee_rates)?;
        let maker_fee_settlement = div_128_scaled(maker_fee, settlement_token_price_i128)?;
        let taker_fee_settlement = div_128_scaled(taker_fee, settlement_token_price_i128)?;
        let maker_fee_token = fee_token_override(&trade.maker_order.order_category, settlement_token_address);
        let taker_fee_token = fee_token_override(&trade.taker_order.order_category, settlement_token_address);
        let maker_uses_fee_token = maker_fee_token.is_some();
        let taker_uses_fee_token = taker_fee_token.is_some();
        let fee_token_address = resolve_fee_token_address(maker_fee_token, taker_fee_token)?;
        let fee_token_price_i128 = if let Some(token_address) = fee_token_address {
            Some(resolve_fee_token_price(token_address, maker_fee_token, taker_fee_token, &maker_state, &taker_state)?)
        } else {
            None
        };
        let maker_fee_token_deduction = if maker_uses_fee_token {
            div_128_scaled(
                maker_fee_settlement,
                fee_token_price_i128
                    .ok_or_else(|| ExecutionError::ExecutionFailed("missing fee token price".to_string()))?,
            )?
        } else {
            0
        };
        let taker_fee_token_deduction = if taker_uses_fee_token {
            div_128_scaled(
                taker_fee_settlement,
                fee_token_price_i128
                    .ok_or_else(|| ExecutionError::ExecutionFailed("missing fee token price".to_string()))?,
            )?
        } else {
            0
        };

        if is_risky_after_trade_loaded(
            ctx,
            state,
            contract,
            trade.maker_order.account,
            settlement_token_address,
            RiskOverrides::spot(
                spot_asset.token_address,
                maker_base_before,
                maker_base_before + maker_base_delta,
                maker_updated_settlement,
            ),
            maker_fee,
            maker_state.clone(),
        )? {
            emit_settle_trade_failed(ctx, ErrorCodes::trade_order_risky(), Errors::trade_order_risky_maker(), trade);
            ctx.set_retdata(vec![Felt::ZERO]);
            ctx.fail("maker risky".to_string());
            return Ok(());
        }

        if is_risky_after_trade_loaded(
            ctx,
            state,
            contract,
            trade.taker_order.account,
            settlement_token_address,
            RiskOverrides::spot(
                spot_asset.token_address,
                taker_base_before,
                taker_base_before + taker_base_delta,
                taker_updated_settlement,
            ),
            taker_fee,
            taker_state.clone(),
        )? {
            emit_settle_trade_failed(ctx, ErrorCodes::trade_order_risky(), Errors::trade_order_risky_taker(), trade);
            ctx.set_retdata(vec![Felt::ZERO]);
            ctx.fail("taker risky".to_string());
            return Ok(());
        }

        if let Some(token_address) = fee_token_address {
            if maker_fee_token_deduction != 0
                && token_amount_from_state(&maker_state, token_address) < maker_fee_token_deduction
            {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::trade_maker_not_enough_fee_token(),
                    Errors::trade_maker_not_enough_fee_token(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("maker not enough fee token".to_string());
                return Ok(());
            }
            if taker_fee_token_deduction != 0
                && token_amount_from_state(&taker_state, token_address) < taker_fee_token_deduction
            {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::trade_taker_not_enough_fee_token(),
                    Errors::trade_taker_not_enough_fee_token(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("taker not enough fee token".to_string());
                return Ok(());
            }
        }

        update_token_balance(
            ctx,
            state,
            contract,
            trade.taker_order.account,
            spot_asset.token_address,
            taker_base_delta,
        )?;
        update_token_balance(
            ctx,
            state,
            contract,
            trade.taker_order.account,
            settlement_token_address,
            taker_quote_delta,
        )?;
        update_token_balance(
            ctx,
            state,
            contract,
            trade.maker_order.account,
            spot_asset.token_address,
            maker_base_delta,
        )?;
        update_token_balance(
            ctx,
            state,
            contract,
            trade.maker_order.account,
            settlement_token_address,
            maker_quote_delta,
        )?;

        let _post_fee_balances = fee_payments(
            ctx,
            state,
            contract,
            caller,
            trade.id,
            settlement_token_address,
            fee_token_address,
            fee_token_price_i128,
            FeePaymentSide {
                account: trade.maker_order.account,
                settlement_after_trade: maker_updated_settlement,
                fee_in_settlement: maker_fee_settlement,
                fee_token_deduction: maker_fee_token_deduction,
                payment_token_address: maker_fee_token.unwrap_or(settlement_token_address),
                pays_in_fee_token: maker_uses_fee_token,
            },
            FeePaymentSide {
                account: trade.taker_order.account,
                settlement_after_trade: taker_updated_settlement,
                fee_in_settlement: taker_fee_settlement,
                fee_token_deduction: taker_fee_token_deduction,
                payment_token_address: taker_fee_token.unwrap_or(settlement_token_address),
                pays_in_fee_token: taker_uses_fee_token,
            },
        )?;

        emit_token_balance_update(
            ctx,
            trade.maker_order.account,
            spot_asset.token_address,
            i128_to_felt(maker_base_before),
            i128_to_felt(maker_base_before + maker_base_delta),
            Felt::ZERO,
        );
        emit_token_balance_update(
            ctx,
            trade.taker_order.account,
            spot_asset.token_address,
            i128_to_felt(taker_base_before),
            i128_to_felt(taker_base_before + taker_base_delta),
            Felt::ZERO,
        );

        emit_token_balance_update(
            ctx,
            trade.maker_order.account,
            settlement_token_address,
            i128_to_felt(maker_settlement_before),
            i128_to_felt(maker_updated_settlement),
            Felt::ZERO,
        );
        emit_token_balance_update(
            ctx,
            trade.taker_order.account,
            settlement_token_address,
            i128_to_felt(taker_settlement_before),
            i128_to_felt(taker_updated_settlement),
            Felt::ZERO,
        );

        emit_trade_settled(ctx, trade.id, trade.maker_order.market, trade.price, trade.size);
        let _ = settlement_token_price;
        Ok(())
    })
}

fn settle_perpetual<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
    settlement_token_price: Felt,
    oracle_address: ContractAddress,
    asset_kind: Felt,
    caller: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    time_inner(InnerTimingField::SettlePerpetual, || {
        let (
            settlement_token_address,
            market,
            funding_index_i128,
            settlement_price_i128,
            trade_size_i128,
            trade_price_i128,
            mode,
        ) = {
            let settlement_token_address = read_settlement_token_address(ctx, state, contract)?;
            let market = trade.maker_order.market;
            let funding_index = oracle::read_funding_index(ctx, state, oracle_address, market)?;
            let funding_index_i128 = felt_to_i128(funding_index)?;
            let settlement_price_i128 = felt_to_i128(settlement_token_price)?;
            let trade_size_i128 = felt_to_i128(trade.size)?;
            let trade_price_i128 = felt_to_i128(trade.price)?;
            let mode = resolve_perp_storage_mode(ctx, state, contract, market)?;
            (
                settlement_token_address,
                market,
                funding_index_i128,
                settlement_price_i128,
                trade_size_i128,
                trade_price_i128,
                mode,
            )
        };
        let maker_state =
            load_account_state_for_risk(ctx, state, contract, trade.maker_order.account, settlement_token_address)?;
        let taker_state =
            load_account_state_for_risk(ctx, state, contract, trade.taker_order.account, settlement_token_address)?;
        let maker_fee = base_fee_perp(ctx, state, contract, trade, true, mode, asset_kind, &maker_state.fee_rates)?;
        let taker_fee = base_fee_perp(ctx, state, contract, trade, false, mode, asset_kind, &taker_state.fee_rates)?;
        let maker_fee_settlement = div_128_scaled(maker_fee, settlement_price_i128)?;
        let taker_fee_settlement = div_128_scaled(taker_fee, settlement_price_i128)?;
        let maker_fee_token = fee_token_override(&trade.maker_order.order_category, settlement_token_address);
        let taker_fee_token = fee_token_override(&trade.taker_order.order_category, settlement_token_address);
        let maker_uses_fee_token = maker_fee_token.is_some();
        let taker_uses_fee_token = taker_fee_token.is_some();
        let fee_token_address = resolve_fee_token_address(maker_fee_token, taker_fee_token)?;
        let fee_token_price_i128 = if let Some(token_address) = fee_token_address {
            Some(resolve_fee_token_price(token_address, maker_fee_token, taker_fee_token, &maker_state, &taker_state)?)
        } else {
            None
        };
        let maker_fee_token_deduction = if maker_uses_fee_token {
            div_128_scaled(
                maker_fee_settlement,
                fee_token_price_i128
                    .ok_or_else(|| ExecutionError::ExecutionFailed("missing fee token price".to_string()))?,
            )?
        } else {
            0
        };
        let taker_fee_token_deduction = if taker_uses_fee_token {
            div_128_scaled(
                taker_fee_settlement,
                fee_token_price_i128
                    .ok_or_else(|| ExecutionError::ExecutionFailed("missing fee token price".to_string()))?,
            )?
        } else {
            0
        };

        let (mut maker_balance, mut taker_balance) =
            { (perp_balance_from_state(&maker_state, market), perp_balance_from_state(&taker_state, market)) };
        if maker_balance.market == Felt::ZERO {
            maker_balance.market = market;
        }
        if taker_balance.market == Felt::ZERO {
            taker_balance.market = market;
        }

        {
            if trade.maker_order.is_reduce_only
                && reduce_only_will_increase(&maker_balance, trade_size_i128, trade.maker_order.side)?
            {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::reduce_only_will_increase_maker(),
                    Errors::reduce_only_will_increase_maker(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("reduce-only maker".to_string());
                return Ok(());
            }
            if trade.taker_order.is_reduce_only
                && reduce_only_will_increase(&taker_balance, trade_size_i128, trade.taker_order.side)?
            {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::reduce_only_will_increase_taker(),
                    Errors::reduce_only_will_increase_taker(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("reduce-only taker".to_string());
                return Ok(());
            }
        }

        let (
            maker_updated,
            maker_realized_funding,
            maker_realized_pnl,
            taker_updated,
            taker_realized_funding,
            taker_realized_pnl,
        ) = {
            let (maker_updated, maker_realized_funding, maker_realized_pnl) = position_after_trade(
                &maker_balance,
                trade_price_i128,
                trade_size_i128,
                trade.maker_order.side,
                funding_index_i128,
                settlement_price_i128,
            )?;
            let (taker_updated, taker_realized_funding, taker_realized_pnl) = position_after_trade(
                &taker_balance,
                trade_price_i128,
                trade_size_i128,
                trade.taker_order.side,
                funding_index_i128,
                settlement_price_i128,
            )?;
            (
                maker_updated,
                maker_realized_funding,
                maker_realized_pnl,
                taker_updated,
                taker_realized_funding,
                taker_realized_pnl,
            )
        };

        let (maker_settlement_before, taker_settlement_before) = {
            (
                token_amount_from_state(&maker_state, settlement_token_address),
                token_amount_from_state(&taker_state, settlement_token_address),
            )
        };

        let maker_settlement_after = maker_settlement_before + maker_realized_funding + maker_realized_pnl;
        let taker_settlement_after = taker_settlement_before + taker_realized_funding + taker_realized_pnl;

        {
            if is_risky_after_trade_loaded(
                ctx,
                state,
                contract,
                trade.maker_order.account,
                settlement_token_address,
                RiskOverrides::perp(
                    market,
                    felt_to_i128(maker_balance.amount)?,
                    felt_to_i128(maker_updated.amount)?,
                    maker_updated.cost,
                    maker_updated.cached_funding,
                    maker_settlement_after,
                ),
                maker_fee,
                maker_state.clone(),
            )? {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::trade_order_risky(),
                    Errors::trade_order_risky_maker(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("maker risky".to_string());
                return Ok(());
            }

            if is_risky_after_trade_loaded(
                ctx,
                state,
                contract,
                trade.taker_order.account,
                settlement_token_address,
                RiskOverrides::perp(
                    market,
                    felt_to_i128(taker_balance.amount)?,
                    felt_to_i128(taker_updated.amount)?,
                    taker_updated.cost,
                    taker_updated.cached_funding,
                    taker_settlement_after,
                ),
                taker_fee,
                taker_state.clone(),
            )? {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::trade_order_risky(),
                    Errors::trade_order_risky_taker(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("taker risky".to_string());
                return Ok(());
            }
        }

        if let Some(token_address) = fee_token_address {
            if maker_fee_token_deduction != 0
                && token_amount_from_state(&maker_state, token_address) < maker_fee_token_deduction
            {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::trade_maker_not_enough_fee_token(),
                    Errors::trade_maker_not_enough_fee_token(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("maker not enough fee token".to_string());
                return Ok(());
            }
            if taker_fee_token_deduction != 0
                && token_amount_from_state(&taker_state, token_address) < taker_fee_token_deduction
            {
                emit_settle_trade_failed(
                    ctx,
                    ErrorCodes::trade_taker_not_enough_fee_token(),
                    Errors::trade_taker_not_enough_fee_token(),
                    trade,
                );
                ctx.set_retdata(vec![Felt::ZERO]);
                ctx.fail("taker not enough fee token".to_string());
                return Ok(());
            }
        }

        {
            upsert_perpetual_balance(ctx, state, contract, trade.maker_order.account, market, &maker_updated, mode)?;
            upsert_perpetual_balance(ctx, state, contract, trade.taker_order.account, market, &taker_updated, mode)?;

            upsert_total_realized_pnl_and_funding(
                ctx,
                state,
                contract,
                market,
                trade.maker_order.account,
                maker_realized_pnl,
                maker_realized_funding,
                mode,
            )?;
            upsert_total_realized_pnl_and_funding(
                ctx,
                state,
                contract,
                market,
                trade.taker_order.account,
                taker_realized_pnl,
                taker_realized_funding,
                mode,
            )?;

            update_token_balance(
                ctx,
                state,
                contract,
                trade.maker_order.account,
                settlement_token_address,
                maker_realized_funding + maker_realized_pnl,
            )?;
            update_token_balance(
                ctx,
                state,
                contract,
                trade.taker_order.account,
                settlement_token_address,
                taker_realized_funding + taker_realized_pnl,
            )?;
        }

        let _post_fee_balances = {
            fee_payments(
                ctx,
                state,
                contract,
                caller,
                trade.id,
                settlement_token_address,
                fee_token_address,
                fee_token_price_i128,
                FeePaymentSide {
                    account: trade.maker_order.account,
                    settlement_after_trade: maker_settlement_after,
                    fee_in_settlement: maker_fee_settlement,
                    fee_token_deduction: maker_fee_token_deduction,
                    payment_token_address: maker_fee_token.unwrap_or(settlement_token_address),
                    pays_in_fee_token: maker_uses_fee_token,
                },
                FeePaymentSide {
                    account: trade.taker_order.account,
                    settlement_after_trade: taker_settlement_after,
                    fee_in_settlement: taker_fee_settlement,
                    fee_token_deduction: taker_fee_token_deduction,
                    payment_token_address: taker_fee_token.unwrap_or(settlement_token_address),
                    pays_in_fee_token: taker_uses_fee_token,
                },
            )?
        };

        {
            emit_perpetual_balance_update(
                ctx,
                trade.maker_order.account,
                market,
                maker_updated.amount,
                maker_updated.cost,
                trade.id,
                i128_to_felt(trade_size_i128 * get_sign_from_side(trade.maker_order.side)?),
                trade.price,
                i128_to_felt(maker_realized_pnl),
                i128_to_felt(maker_realized_funding),
                maker_updated.cached_funding,
                maker_balance.cached_funding,
                trade.traded_at,
                settlement_token_price,
            );

            emit_perpetual_balance_update(
                ctx,
                trade.taker_order.account,
                market,
                taker_updated.amount,
                taker_updated.cost,
                trade.id,
                i128_to_felt(trade_size_i128 * get_sign_from_side(trade.taker_order.side)?),
                trade.price,
                i128_to_felt(taker_realized_pnl),
                i128_to_felt(taker_realized_funding),
                taker_updated.cached_funding,
                taker_balance.cached_funding,
                trade.traded_at,
                settlement_token_price,
            );

            emit_token_balance_update(
                ctx,
                trade.maker_order.account,
                settlement_token_address,
                i128_to_felt(maker_settlement_before),
                i128_to_felt(maker_settlement_after),
                Felt::ZERO,
            );
            emit_token_balance_update(
                ctx,
                trade.taker_order.account,
                settlement_token_address,
                i128_to_felt(taker_settlement_before),
                i128_to_felt(taker_settlement_after),
                Felt::ZERO,
            );

            emit_trade_settled(ctx, trade.id, market, trade.price, trade.size);
        }
        Ok(())
    })
}

pub fn decode_trade_request_v3(input: &[Felt]) -> Result<(TradeRequestV3, &[Felt]), ExecutionError> {
    let (id, rest) = take_felt(input)?;
    let (size, rest) = take_felt(rest)?;
    let (price, rest) = take_felt(rest)?;
    let (traded_at, rest) = take_felt(rest)?;
    let (maker_order, rest) = decode_order_v3(rest)?;
    let (taker_order, rest) = decode_order_v3(rest)?;

    Ok((TradeRequestV3 { id, size, price, traded_at, maker_order, taker_order }, rest))
}

pub fn extract_trade_markets_v3(input: &[Felt]) -> Result<Vec<Felt>, ExecutionError> {
    let (trade, _remaining) = decode_trade_request_v3(input)?;
    Ok(vec![trade.maker_order.market, trade.taker_order.market])
}

pub fn extract_trade_request_v3(input: &[Felt]) -> Result<TradeRequestV3, ExecutionError> {
    let (trade, _remaining) = decode_trade_request_v3(input)?;
    Ok(trade)
}

pub fn extract_account_perp_markets_for_trace<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<Vec<Felt>, ExecutionError> {
    let mut ctx = ExecutionContext::new();
    let mode = resolve_perp_storage_mode_for_account(&mut ctx, state, contract, account)?;
    let mut markets = Vec::new();
    let tail_key = perp_balance_tail_key(mode, account);
    let mut current = ctx.storage_read(state, contract, tail_key)?;
    while current != Felt::ZERO {
        markets.push(current);
        let base = resolve_perp_balance_base(state, &mut ctx, contract, account, current)?;
        current = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    }
    Ok(markets)
}

fn decode_order_v3(input: &[Felt]) -> Result<(OrderV3, &[Felt]), ExecutionError> {
    let (account_felt, rest) = take_felt(input)?;
    let (market, rest) = take_felt(rest)?;
    let (side, rest) = take_felt(rest)?;
    let (order_type, rest) = take_felt(rest)?;
    let (size, rest) = take_felt(rest)?;
    let (price, rest) = take_felt(rest)?;
    let (signature_timestamp, rest) = take_felt(rest)?;
    let (is_reduce_only, rest) = take_bool(rest)?;
    let (order_category, rest) = decode_order_category(rest)?;

    Ok((
        OrderV3 {
            account: ContractAddress(account_felt),
            market,
            side,
            orderType: order_type,
            size,
            price,
            signature_timestamp,
            is_reduce_only,
            order_category,
        },
        rest,
    ))
}

#[cfg(any(test, feature = "testing"))]
pub fn decode_order_v3_for_test(input: &[Felt]) -> Result<(OrderV3, &[Felt]), ExecutionError> {
    decode_order_v3(input)
}

#[cfg(any(test, feature = "testing"))]
pub fn decode_order_category_for_test(input: &[Felt]) -> Result<(OrderCategory, &[Felt]), ExecutionError> {
    decode_order_category(input)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn encode_trade_request_v3_for_test(trade: &TradeRequestV3) -> Vec<Felt> {
    encode_trade_request_v3(trade)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn encode_order_v3_for_test(order: &OrderV3) -> Vec<Felt> {
    encode_order_v3(order)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn encode_order_category_for_test(cat: &OrderCategory) -> Vec<Felt> {
    encode_order_category(cat)
}

#[cfg(test)]
pub(crate) fn perp_map_key_for_test(mode: u8, name: &str, key: Felt) -> StorageKey {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    perp_map_key(mode, name, key)
}

#[cfg(test)]
pub(crate) fn perp_map2_key_for_test(mode: u8, name: &str, key1: Felt, key2: Felt) -> StorageKey {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    perp_map2_key(mode, name, key1, key2)
}

#[cfg(test)]
pub(crate) fn perp_var_key_for_test(mode: u8, name: &str) -> StorageKey {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    perp_var_key(mode, name)
}

#[cfg(test)]
pub(crate) fn resolve_perp_storage_mode_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
) -> u8 {
    match resolve_perp_storage_mode(ctx, state, contract, market) {
        Ok(mode) => mode_to_u8(mode),
        Err(_) => 255,
    }
}

#[cfg(test)]
pub(crate) fn resolve_perp_balance_base_for_test<S: StateReader>(
    state: &S,
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
) -> Result<StorageKey, ExecutionError> {
    resolve_perp_balance_base(state, ctx, contract, account, market)
}

#[cfg(test)]
pub(crate) fn read_perpetual_balance_fields_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    mode: u8,
) -> Result<(Felt, Felt, Felt, Felt, Felt, Felt), ExecutionError> {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    let balance = read_perpetual_balance(ctx, state, contract, account, market, mode)?;
    Ok((balance.market, balance.amount, balance.cost, balance.cached_funding, balance.prev, balance.next))
}

#[cfg(test)]
pub(crate) fn upsert_perpetual_balance_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    amount: i128,
    cost: i128,
    cached_funding: i128,
    mode: u8,
) -> Result<(), ExecutionError> {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    let balance = PerpetualBalance {
        market,
        amount: i128_to_felt(amount),
        cost: i128_to_felt(cost),
        cached_funding: i128_to_felt(cached_funding),
        prev: Felt::ZERO,
        next: Felt::ZERO,
    };
    upsert_perpetual_balance(ctx, state, contract, account, market, &balance, mode)
}

#[cfg(test)]
pub(crate) fn ensure_perpetual_balance_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    mode: u8,
) -> Result<(), ExecutionError> {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    ensure_perpetual_balance(ctx, state, contract, account, market, mode)
}

#[cfg(test)]
pub(crate) fn remove_perpetual_balance_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    mode: u8,
) -> Result<(), ExecutionError> {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    remove_perpetual_balance(ctx, state, contract, account, market, mode)
}

#[cfg(test)]
pub(crate) fn upsert_total_realized_pnl_and_funding_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    account: ContractAddress,
    realized_pnl: i128,
    realized_funding: i128,
    mode: u8,
) -> Result<(), ExecutionError> {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    upsert_total_realized_pnl_and_funding(ctx, state, contract, market, account, realized_pnl, realized_funding, mode)
}

#[cfg(test)]
pub(crate) fn trade_support_fee_v2_for_test(trade: &TradeRequestV3) -> bool {
    trade_support_fee_v2(trade)
}

#[cfg(test)]
pub(crate) fn apply_referral_discount_for_test(
    fee: i128,
    referrer: ContractAddress,
    fee_discount: i128,
    fee_commission: i128,
) -> Result<i128, ExecutionError> {
    let referral = AccountReferralLite { referrer, fee_commission, fee_discount };
    apply_referral_discount(fee, &referral)
}

#[cfg(test)]
pub(crate) fn is_risky_after_trade_spot_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    settlement_token_address: ContractAddress,
    token_address: ContractAddress,
    before: i128,
    after: i128,
    settlement_after: i128,
    trade_fee: i128,
) -> Result<bool, ExecutionError> {
    is_risky_after_trade(
        ctx,
        state,
        contract,
        account,
        settlement_token_address,
        RiskOverrides::spot(token_address, before, after, settlement_after),
        trade_fee,
    )
}

#[cfg(test)]
pub(crate) fn is_risky_after_trade_perp_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    settlement_token_address: ContractAddress,
    market: Felt,
    before: i128,
    after: i128,
    cost: Felt,
    cached_funding: Felt,
    settlement_after: i128,
    trade_fee: i128,
) -> Result<bool, ExecutionError> {
    is_risky_after_trade(
        ctx,
        state,
        contract,
        account,
        settlement_token_address,
        RiskOverrides::perp(market, before, after, cost, cached_funding, settlement_after),
        trade_fee,
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn get_value_for_test(amount: i128, mark_price: i128) -> Result<i128, ExecutionError> {
    let balance = PerpetualBalance {
        market: Felt::ZERO,
        amount: i128_to_felt(amount),
        cost: Felt::ZERO,
        cached_funding: Felt::ZERO,
        prev: Felt::ZERO,
        next: Felt::ZERO,
    };
    get_value(&balance, mark_price)
}

#[cfg(test)]
pub(crate) fn unrealized_pnl_for_test(
    amount: i128,
    cost: i128,
    mark_price: i128,
    settlement_price: i128,
) -> Result<i128, ExecutionError> {
    let balance = PerpetualBalance {
        market: Felt::ZERO,
        amount: i128_to_felt(amount),
        cost: i128_to_felt(cost),
        cached_funding: Felt::ZERO,
        prev: Felt::ZERO,
        next: Felt::ZERO,
    };
    unrealized_pnl(&balance, mark_price, settlement_price)
}

#[cfg(test)]
pub(crate) fn unrealized_funding_pnl_for_test(
    amount: i128,
    cached_funding: i128,
    funding_index: i128,
) -> Result<i128, ExecutionError> {
    let balance = PerpetualBalance {
        market: Felt::ZERO,
        amount: i128_to_felt(amount),
        cost: Felt::ZERO,
        cached_funding: i128_to_felt(cached_funding),
        prev: Felt::ZERO,
        next: Felt::ZERO,
    };
    unrealized_funding_pnl(&balance, funding_index, MULTIPLIER)
}

#[cfg(test)]
pub(crate) fn get_asset_value_for_test(token_amounts: &[i128], token_prices: &[i128]) -> Result<i128, ExecutionError> {
    if token_amounts.len() != token_prices.len() {
        return Err(ExecutionError::ExecutionFailed("token length mismatch".to_string()));
    }
    let mut balances = Vec::with_capacity(token_amounts.len());
    for (idx, amount) in token_amounts.iter().enumerate() {
        balances.push(TokenBalanceLite {
            token_address: ContractAddress(Felt::from((idx + 1) as u64)),
            token_name: Felt::ZERO,
            amount: *amount,
        });
    }
    let state = AccountStateLite {
        token_balances: balances,
        token_prices: token_prices.to_vec(),
        settlement_token_index: 0,
        perp_balances: Vec::new(),
        perp_prices: Vec::new(),
        perp_funding_indices: Vec::new(),
        perp_imf_base: Vec::new(),
        perp_option_assets: Vec::new(),
        spot_prices: HashMap::new(),
        option_margin_params: HashMap::new(),
        perp_fee_provision_rate: Vec::new(),
        perp_asset_kinds: Vec::new(),
        fee_rates: FeeRatesLite::zero(),
        referral: AccountReferralLite { referrer: ContractAddress(Felt::ZERO), fee_commission: 0, fee_discount: 0 },
        margin_methodology: MarginMethodologyLite::CrossMargin,
        settlement_token_price: 0,
        perp_mmf_factor: 0,
    };
    get_asset_value(&state)
}

#[cfg(test)]
pub(crate) fn margin_requirement_and_total_upnl_for_test(
    token_amounts: &[i128],
    token_prices: &[i128],
    settlement_token_index: usize,
    perp_amounts: &[i128],
    perp_costs: &[i128],
    perp_cached_funding: &[i128],
    perp_prices: &[i128],
    perp_funding_indices: &[i128],
    perp_imf_base: &[i128],
    perp_fee_provision_rate: &[Option<i128>],
    fee_rate_taker: i128,
    referrer: ContractAddress,
    fee_commission: i128,
    fee_discount: i128,
    margin_methodology: u8,
    settlement_token_price: i128,
    perp_mmf_factor: i128,
    include_tokens: bool,
) -> Result<(i128, i128), ExecutionError> {
    if token_amounts.len() != token_prices.len() {
        return Err(ExecutionError::ExecutionFailed("token length mismatch".to_string()));
    }
    if perp_amounts.len() != perp_costs.len()
        || perp_amounts.len() != perp_cached_funding.len()
        || perp_amounts.len() != perp_prices.len()
        || perp_amounts.len() != perp_funding_indices.len()
        || perp_amounts.len() != perp_imf_base.len()
        || perp_amounts.len() != perp_fee_provision_rate.len()
    {
        return Err(ExecutionError::ExecutionFailed("perp length mismatch".to_string()));
    }

    let mut token_balances = Vec::with_capacity(token_amounts.len());
    for (idx, amount) in token_amounts.iter().enumerate() {
        token_balances.push(TokenBalanceLite {
            token_address: ContractAddress(Felt::from((idx + 1) as u64)),
            token_name: Felt::ZERO,
            amount: *amount,
        });
    }

    let mut perp_balances = Vec::with_capacity(perp_amounts.len());
    for i in 0..perp_amounts.len() {
        perp_balances.push(PerpetualBalance {
            market: Felt::from((i + 1) as u64),
            amount: i128_to_felt(perp_amounts[i]),
            cost: i128_to_felt(perp_costs[i]),
            cached_funding: i128_to_felt(perp_cached_funding[i]),
            prev: Felt::ZERO,
            next: Felt::ZERO,
        });
    }

    let state = AccountStateLite {
        token_balances,
        token_prices: token_prices.to_vec(),
        settlement_token_index,
        perp_balances,
        perp_prices: perp_prices.to_vec(),
        perp_funding_indices: perp_funding_indices.to_vec(),
        perp_imf_base: perp_imf_base.to_vec(),
        perp_option_assets: vec![None; perp_amounts.len()],
        spot_prices: HashMap::new(),
        option_margin_params: HashMap::new(),
        perp_fee_provision_rate: perp_fee_provision_rate
            .iter()
            .map(|rate| FeeWithCapLite { fee: rate.unwrap_or(0), fee_cap: 0, fee_floor: 0 })
            .collect(),
        perp_asset_kinds: vec![assets_manager::asset_kind_future(); perp_amounts.len()],
        fee_rates: FeeRatesLite::from_taker(fee_rate_taker),
        referral: AccountReferralLite { referrer, fee_commission, fee_discount },
        margin_methodology: if margin_methodology == 0 {
            MarginMethodologyLite::CrossMargin
        } else {
            MarginMethodologyLite::PortfolioMargin
        },
        settlement_token_price,
        perp_mmf_factor,
    };

    margin_requirement_and_total_upnl(&state, include_tokens)
}

#[cfg(test)]
pub(crate) fn read_token_balance_amount_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
) -> Result<i128, ExecutionError> {
    read_token_balance_amount(ctx, state, contract, account, token_address)
}

#[cfg(test)]
pub(crate) fn upsert_token_balance_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
    delta: i128,
) -> Result<(), ExecutionError> {
    upsert_token_balance(ctx, state, contract, account, token_address, delta)
}

#[cfg(test)]
pub(crate) fn update_token_balance_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
    delta: i128,
) -> Result<(), ExecutionError> {
    update_token_balance(ctx, state, contract, account, token_address, delta)
}

#[cfg(test)]
pub(crate) fn get_asset_kind_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    assets_manager_address: ContractAddress,
    market: Felt,
) -> Result<Felt, ExecutionError> {
    get_asset_kind(ctx, state, contract, assets_manager_address, market)
}

#[cfg(test)]
pub(crate) fn read_settlement_token_address_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
) -> Result<ContractAddress, ExecutionError> {
    read_settlement_token_address(ctx, state, contract)
}

#[cfg(test)]
pub(crate) fn read_settlement_token_name_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
) -> Result<Felt, ExecutionError> {
    read_settlement_token_name(ctx, state, contract)
}

#[cfg(test)]
pub(crate) fn read_settlement_token_price_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    oracle_address: ContractAddress,
    asset_key: Felt,
) -> Result<Felt, ExecutionError> {
    oracle::read_settlement_token_price(ctx, state, oracle_address, asset_key)
}

#[cfg(test)]
pub(crate) fn read_contract_address_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    key: StorageKey,
) -> Result<ContractAddress, ExecutionError> {
    read_contract_address(ctx, state, contract, key)
}

#[cfg(test)]
pub(crate) fn resolve_market_delegate_address_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
) -> Result<Option<ContractAddress>, ExecutionError> {
    resolve_market_delegate_address(ctx, state, contract, market)
}

#[cfg(test)]
pub(crate) fn resolve_token_balance_base_for_test<S: StateReader>(
    state: &S,
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    account: ContractAddress,
    token: ContractAddress,
) -> Result<StorageKey, ExecutionError> {
    resolve_token_balance_base(state, ctx, contract, account, token)
}

#[cfg(test)]
pub(crate) fn read_max_pending_transfer_executions_for_test<S: StateReader>(
    state: &S,
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
) -> Result<u64, ExecutionError> {
    read_max_pending_transfer_executions(state, ctx, contract)
}

#[cfg(test)]
pub(crate) fn add_pending_transfer_for_test<S: StateReader>(
    state: &S,
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    executor: ContractAddress,
    trade_id: Felt,
    recipient: ContractAddress,
    token_address: ContractAddress,
    amount: i128,
) -> Result<(), ExecutionError> {
    add_pending_transfer(state, ctx, contract, executor, trade_id, recipient, token_address, amount)
}

#[cfg(test)]
pub(crate) fn fee_rate_v2_for_order_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    category: &OrderCategory,
    is_maker: bool,
    mode: u8,
) -> Result<Option<i128>, ExecutionError> {
    let mode = match mode {
        0 => PerpStorageMode::LegacyPedersen,
        1 => PerpStorageMode::LegacyPoseidon,
        2 => PerpStorageMode::Substorage,
        3 => PerpStorageMode::SubstorageAdd,
        4 => PerpStorageMode::SubstorageHash,
        _ => PerpStorageMode::LegacyPedersen,
    };
    fee_rate_v2_for_order(ctx, state, contract, market, category, is_maker, mode)
}

#[cfg(test)]
pub(crate) fn base_fee_spot_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
    is_maker: bool,
) -> Result<i128, ExecutionError> {
    let account = if is_maker { trade.maker_order.account } else { trade.taker_order.account };
    let fee_rates = read_fee_rates_for_account(ctx, state, contract, account)?;
    base_fee_spot(ctx, state, contract, trade, is_maker, &fee_rates)
}

#[cfg(test)]
pub(crate) fn base_fee_perp_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
    is_maker: bool,
) -> Result<i128, ExecutionError> {
    let mode = resolve_perp_storage_mode(ctx, state, contract, trade.maker_order.market)?;
    let account = if is_maker { trade.maker_order.account } else { trade.taker_order.account };
    let fee_rates = read_fee_rates_for_account(ctx, state, contract, account)?;
    let assets_manager_address = read_contract_address(ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;
    let asset_kind = get_asset_kind(ctx, state, contract, assets_manager_address, trade.maker_order.market)?;
    base_fee_perp(ctx, state, contract, trade, is_maker, mode, asset_kind, &fee_rates)
}

#[cfg(test)]
pub(crate) fn enforce_max_assets_per_account_for_test<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
) -> Result<(), ExecutionError> {
    enforce_max_assets_per_account(ctx, state, contract, trade)
}

fn decode_order_category(input: &[Felt]) -> Result<(OrderCategory, &[Felt]), ExecutionError> {
    let (tag, rest) = take_felt(input)?;
    let tag_u32 = felt_to_u32(tag)?;

    match tag_u32 {
        0 => Ok((OrderCategory::Unspecified, rest)),
        1 => Ok((OrderCategory::API, rest)),
        2 => Ok((OrderCategory::RPI, rest)),
        3 => Ok((OrderCategory::Interactive, rest)),
        4 => {
            let (fee, rest) = take_felt(rest)?;
            let (fee_cap, rest) = take_felt(rest)?;
            let (fee_floor, rest) = take_felt(rest)?;
            Ok((OrderCategory::Dynamic(FeeWithCapRequest { fee, fee_cap, fee_floor }), rest))
        }
        5 => {
            let (fee, rest) = take_felt(rest)?;
            let (fee_cap, rest) = take_felt(rest)?;
            let (fee_floor, rest) = take_felt(rest)?;
            let (fee_token, rest) = take_felt(rest)?;
            Ok((
                OrderCategory::DynamicWithToken(
                    crate::contracts::paradex::schema::paraclear_types::FeeWithCapRequestV2 {
                        fee,
                        fee_cap,
                        fee_floor,
                        fee_token: ContractAddress(fee_token),
                    },
                ),
                rest,
            ))
        }
        _ => Err(ExecutionError::ExecutionFailed("invalid OrderCategory tag".to_string())),
    }
}

fn take_felt(input: &[Felt]) -> Result<(Felt, &[Felt]), ExecutionError> {
    if input.is_empty() {
        return Err(ExecutionError::ExecutionFailed("calldata underflow".to_string()));
    }
    Ok((input[0], &input[1..]))
}

fn take_bool(input: &[Felt]) -> Result<(bool, &[Felt]), ExecutionError> {
    let (value, rest) = take_felt(input)?;
    if value == Felt::ZERO {
        Ok((false, rest))
    } else if value == Felt::ONE {
        Ok((true, rest))
    } else {
        Err(ExecutionError::ExecutionFailed("invalid bool".to_string()))
    }
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

fn felt_to_u64(value: Felt) -> Result<u64, ExecutionError> {
    let bytes = value.to_bytes_be();
    if bytes[..24].iter().any(|b| *b != 0) {
        return Err(ExecutionError::ExecutionFailed("value too large for u64".to_string()));
    }
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&bytes[24..]);
    Ok(u64::from_be_bytes(arr))
}

fn read_contract_address<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    key: crate::core::types::StorageKey,
) -> Result<ContractAddress, ExecutionError> {
    let value = ctx.storage_read(state, contract, key)?;
    Ok(ContractAddress(value))
}

fn get_asset_kind<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    assets_manager: ContractAddress,
    market: Felt,
) -> Result<Felt, ExecutionError> {
    if market == Felt::ZERO {
        return Ok(assets_manager::ASSET_KIND_UNSUPPORTED);
    }
    if is_perpetual_future_supported(ctx, state, contract, market)? {
        return Ok(assets_manager::asset_kind_future());
    }
    assets_manager::get_asset_kind(ctx, state, assets_manager, market)
}

fn is_perpetual_future_supported<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
) -> Result<bool, ExecutionError> {
    // log_perp_storage_candidates(market);
    let mode = resolve_perp_storage_mode(ctx, state, contract, market)?;
    let base = perp_asset_key(mode, market);
    let stored_market = ctx.storage_read(state, contract, base)?;
    Ok(stored_market == market)
}

fn oracle_settlement_key() -> Felt {
    crate::core::storage::short_string_to_felt("USDC")
}

fn read_settlement_token_address<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
) -> Result<ContractAddress, ExecutionError> {
    // TokenAsset struct layout: initial_weight, maintenance_weight, conversion_weight, tick_size, token_address, token_name
    let base = *layout::PARACLEAR_SETTLEMENT_TOKEN_ASSET_BASE;
    let token_address = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    Ok(ContractAddress(token_address))
}

fn resolve_market_delegate_address<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
) -> Result<Option<ContractAddress>, ExecutionError> {
    let primary_key = paraclear_layout::Paraclear_market_delegate_key(market);
    let primary_value =
        ctx.storage_read(state, contract, primary_key).map_err(|e| ExecutionError::ExecutionFailed(e.to_string()))?;
    if primary_value != Felt::ZERO {
        return Ok(Some(ContractAddress(primary_value)));
    }
    Ok(None)
}

pub fn resolve_market_delegate_for_trace<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    market: Felt,
) -> Result<Option<ContractAddress>, ExecutionError> {
    // Trace helper: read-only. Still route storage reads via ExecutionContext so per-tx
    // instrumentation is exercised the same way as execution.
    let mut ctx = ExecutionContext::new();
    resolve_market_delegate_address(&mut ctx, state, contract, market)
}

fn enforce_max_assets_per_account<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
) -> Result<(), ExecutionError> {
    time_inner(InnerTimingField::EnforceMaxAssetsPerAccount, || {
        let max_assets = read_max_assets_per_account(ctx, state, contract)?;
        if max_assets == 0 {
            return Ok(());
        }

        if !trade.maker_order.is_reduce_only {
            let maker_tokens = count_token_assets(ctx, state, contract, trade.maker_order.account)?;
            let maker_perps = count_perp_assets(ctx, state, contract, trade.maker_order.account)?;
            let maker_assets = maker_tokens + maker_perps;
            if maker_assets > max_assets {
                return Err(ExecutionError::ExecutionFailed("Trade: maker too many assets".to_string()));
            }
        }
        if !trade.taker_order.is_reduce_only {
            let taker_tokens = count_token_assets(ctx, state, contract, trade.taker_order.account)?;
            let taker_perps = count_perp_assets(ctx, state, contract, trade.taker_order.account)?;
            let taker_assets = taker_tokens + taker_perps;
            if taker_assets > max_assets {
                return Err(ExecutionError::ExecutionFailed("Trade: taker too many assets".to_string()));
            }
        }
        Ok(())
    })
}

fn read_max_assets_per_account<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
) -> Result<u64, ExecutionError> {
    let base = *layout::GLOBAL_CONFIGURATION_BASE;
    let value = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    let mut max_assets = felt_to_u64(value)?;
    if max_assets == 0 {
        max_assets = 150;
    }
    Ok(max_assets)
}

fn count_token_assets<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<u64, ExecutionError> {
    time_inner(InnerTimingField::CountTokenAssets, || {
        let mut count = 0u64;
        let inner = token_balance_inner_base(account);
        let tail_key = resolve_token_balance_tail_key(state, contract, account)?;
        let mut current = ctx.storage_read(state, contract, tail_key)?;
        while current != Felt::ZERO {
            count += 1;
            let base = token_balance_base_from_inner(inner, ContractAddress(current));
            current = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
        }
        Ok(count)
    })
}

fn count_perp_assets<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<u64, ExecutionError> {
    time_inner(InnerTimingField::CountPerpAssets, || {
        let mut count = 0u64;
        let mode = resolve_perp_storage_mode_for_account(ctx, state, contract, account)?;
        let tail_key = perp_balance_tail_key(mode, account);
        let inner =
            if matches!(mode, PerpStorageMode::LegacyPedersen) { Some(perp_balance_inner_base(account)) } else { None };
        let mut current = ctx.storage_read(state, contract, tail_key)?;
        while current != Felt::ZERO {
            count += 1;
            let base = if let Some(inner) = inner {
                perp_balance_base_from_inner(inner, current)
            } else {
                resolve_perp_balance_base(state, ctx, contract, account, current)?
            };
            current = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
        }
        Ok(count)
    })
}

#[derive(Clone, Debug)]
struct PerpetualBalance {
    market: Felt,
    amount: Felt,
    cost: Felt,
    cached_funding: Felt,
    prev: Felt,
    next: Felt,
}

#[derive(Clone, Debug)]
struct TokenBalanceLite {
    token_address: ContractAddress,
    token_name: Felt,
    amount: i128,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct AccountReferralLite {
    referrer: ContractAddress,
    fee_commission: i128,
    fee_discount: i128,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
struct FeeRateLite {
    exists: bool,
    maker: i128,
    taker: i128,
}

impl FeeRateLite {
    fn zero() -> Self {
        Self { exists: false, maker: 0, taker: 0 }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
struct SettlementFeeRatesLite {
    futures: FeeRateLite,
    options: FeeRateLite,
}

impl SettlementFeeRatesLite {
    fn zero() -> Self {
        Self { futures: FeeRateLite::zero(), options: FeeRateLite::zero() }
    }
}

#[derive(Clone, Copy, Debug)]
struct FeeRatesLite {
    futures: FeeRateLite,
    options: FeeRateLite,
    spot: FeeRateLite,
    #[allow(dead_code)]
    settlement: SettlementFeeRatesLite,
}

impl FeeRatesLite {
    #[cfg(test)]
    fn zero() -> Self {
        Self {
            futures: FeeRateLite::zero(),
            options: FeeRateLite::zero(),
            spot: FeeRateLite::zero(),
            settlement: SettlementFeeRatesLite::zero(),
        }
    }

    #[cfg(test)]
    fn from_taker(taker: i128) -> Self {
        let rate = FeeRateLite { exists: true, maker: taker, taker };
        Self { futures: rate, options: rate, spot: rate, settlement: SettlementFeeRatesLite::zero() }
    }
}

#[derive(Clone, Copy, Debug)]
enum MarginMethodologyLite {
    CrossMargin,
    PortfolioMargin,
}

#[derive(Clone, Copy, Debug, Default)]
struct FeeWithCapLite {
    fee: i128,
    fee_cap: i128,
    fee_floor: i128,
}

#[derive(Clone, Copy, Debug)]
struct OptionAssetLite {
    base_asset: Felt,
    option_type: Felt,
    strike: i128,
}

#[derive(Clone, Copy, Debug, Default)]
struct OptionMarginParamsLite {
    premium_multiplier: i128,
    long_itm: i128,
    short_itm: i128,
    short_otm: i128,
    short_put_cap: i128,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
struct OptionCrossMarginParamsLite {
    imf: OptionMarginParamsLite,
    mmf: OptionMarginParamsLite,
}

#[derive(Clone, Copy, Debug)]
struct FeePaymentSide {
    account: ContractAddress,
    settlement_after_trade: i128,
    fee_in_settlement: i128,
    fee_token_deduction: i128,
    payment_token_address: ContractAddress,
    pays_in_fee_token: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct AccountStateLite {
    token_balances: Vec<TokenBalanceLite>,
    token_prices: Vec<i128>,
    settlement_token_index: usize,
    perp_balances: Vec<PerpetualBalance>,
    perp_prices: Vec<i128>,
    perp_funding_indices: Vec<i128>,
    perp_imf_base: Vec<i128>,
    perp_option_assets: Vec<Option<OptionAssetLite>>,
    spot_prices: HashMap<Felt, i128>,
    option_margin_params: HashMap<(Felt, Felt), OptionCrossMarginParamsLite>,
    perp_fee_provision_rate: Vec<FeeWithCapLite>,
    perp_asset_kinds: Vec<Felt>,
    fee_rates: FeeRatesLite,
    referral: AccountReferralLite,
    margin_methodology: MarginMethodologyLite,
    settlement_token_price: i128,
    perp_mmf_factor: i128,
}

#[derive(Clone, Debug, Default)]
struct LoadedPerpStateLite {
    balances: Vec<PerpetualBalance>,
    prices: Vec<i128>,
    funding_indices: Vec<i128>,
    imf_base: Vec<i128>,
    option_assets: Vec<Option<OptionAssetLite>>,
    spot_prices: HashMap<Felt, i128>,
    option_margin_params: HashMap<(Felt, Felt), OptionCrossMarginParamsLite>,
    fee_provision_rates: Vec<FeeWithCapLite>,
    asset_kinds: Vec<Felt>,
}

fn token_amount_from_state(state: &AccountStateLite, token_address: ContractAddress) -> i128 {
    state.token_balances.iter().find(|b| b.token_address == token_address).map(|b| b.amount).unwrap_or(0)
}

fn perp_balance_from_state(state: &AccountStateLite, market: Felt) -> PerpetualBalance {
    state.perp_balances.iter().find(|b| b.market == market).cloned().unwrap_or(PerpetualBalance {
        market: Felt::ZERO,
        amount: Felt::ZERO,
        cost: Felt::ZERO,
        cached_funding: Felt::ZERO,
        prev: Felt::ZERO,
        next: Felt::ZERO,
    })
}

struct RiskOverrides {
    spot_token: Option<(ContractAddress, i128, i128)>,
    perp_market: Option<(Felt, i128, i128, Felt, Felt)>,
    settlement_token_amount: Option<i128>,
}

impl RiskOverrides {
    fn spot(token_address: ContractAddress, before: i128, after: i128, settlement_after: i128) -> Self {
        Self {
            spot_token: Some((token_address, before, after)),
            perp_market: None,
            settlement_token_amount: Some(settlement_after),
        }
    }

    fn perp(market: Felt, before: i128, after: i128, cost: Felt, cached_funding: Felt, settlement_after: i128) -> Self {
        Self {
            spot_token: None,
            perp_market: Some((market, before, after, cost, cached_funding)),
            settlement_token_amount: Some(settlement_after),
        }
    }
}

fn update_token_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
    delta: i128,
) -> Result<(), ExecutionError> {
    upsert_token_balance(ctx, state, contract, account, token_address, delta)?;
    Ok(())
}

fn read_token_balance_amount<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
) -> Result<i128, ExecutionError> {
    let base = resolve_token_balance_base(state, ctx, contract, account, token_address)?;
    let amount_key = storage_key_with_offset(base, 1);
    let current = ctx.storage_read(state, contract, amount_key)?;
    felt_to_i128(current)
}

fn upsert_token_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
    delta: i128,
) -> Result<(), ExecutionError> {
    let base = resolve_token_balance_base(state, ctx, contract, account, token_address)?;
    let stored_token = ctx.storage_read(state, contract, base)?;
    if stored_token == Felt::ZERO {
        let tail_key = resolve_token_balance_tail_key(state, contract, account)?;
        let tail = ctx.storage_read(state, contract, tail_key)?;
        let new_balance_amount = i128_to_felt(delta);
        ctx.storage_write(contract, base, token_address.0);
        ctx.storage_write(contract, storage_key_with_offset(base, 1), new_balance_amount);
        ctx.storage_write(contract, storage_key_with_offset(base, 2), tail);
        ctx.storage_write(contract, storage_key_with_offset(base, 3), Felt::ZERO);
        ctx.storage_write(contract, tail_key, token_address.0);
        if tail != Felt::ZERO {
            let tail_base = resolve_token_balance_base(state, ctx, contract, account, ContractAddress(tail))?;
            ctx.storage_write(contract, storage_key_with_offset(tail_base, 3), token_address.0);
        }
        return Ok(());
    }

    let amount_key = storage_key_with_offset(base, 1);
    let current = felt_to_i128(ctx.storage_read(state, contract, amount_key)?)?;
    let updated = current + delta;
    if updated == 0 {
        remove_token_balance(ctx, state, contract, account, token_address)?;
        return Ok(());
    }
    ctx.storage_write(contract, amount_key, i128_to_felt(updated));
    Ok(())
}

fn remove_token_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    token_address: ContractAddress,
) -> Result<(), ExecutionError> {
    let base = resolve_token_balance_base(state, ctx, contract, account, token_address)?;
    let prev = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let next = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;

    if prev != Felt::ZERO {
        let prev_base = resolve_token_balance_base(state, ctx, contract, account, ContractAddress(prev))?;
        ctx.storage_write(contract, storage_key_with_offset(prev_base, 3), next);
    }
    if next != Felt::ZERO {
        let next_base = resolve_token_balance_base(state, ctx, contract, account, ContractAddress(next))?;
        ctx.storage_write(contract, storage_key_with_offset(next_base, 2), prev);
    }

    let tail_key = resolve_token_balance_tail_key(state, contract, account)?;
    let tail = ctx.storage_read(state, contract, tail_key)?;
    if tail == token_address.0 {
        ctx.storage_write(contract, tail_key, prev);
    }

    ctx.storage_write(contract, base, Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 1), Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 2), Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 3), Felt::ZERO);
    Ok(())
}

fn read_perpetual_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    mode: PerpStorageMode,
) -> Result<PerpetualBalance, ExecutionError> {
    let _ = mode;
    let base = resolve_perp_balance_base(state, ctx, contract, account, market)?;
    let market_v = ctx.storage_read(state, contract, base)?;
    let amount = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    let cost = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let cached_funding = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
    let prev = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    let next = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
    Ok(PerpetualBalance { market: market_v, amount, cost, cached_funding, prev, next })
}

fn read_perpetual_balance_with_inner<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    inner: Felt,
    market: Felt,
) -> Result<PerpetualBalance, ExecutionError> {
    let base = perp_balance_base_from_inner(inner, market);
    let market_v = ctx.storage_read(state, contract, base)?;
    let amount = ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?;
    let cost = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
    let cached_funding = ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?;
    let prev = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    let next = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
    Ok(PerpetualBalance { market: market_v, amount, cost, cached_funding, prev, next })
}

fn upsert_perpetual_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    balance: &PerpetualBalance,
    mode: PerpStorageMode,
) -> Result<(), ExecutionError> {
    let _ = mode;
    let base = resolve_perp_balance_base(state, ctx, contract, account, market)?;
    let stored_market = ctx.storage_read(state, contract, base)?;
    if balance.amount == Felt::ZERO {
        if stored_market != Felt::ZERO {
            remove_perpetual_balance(ctx, state, contract, account, market, mode)?;
        }
        return Ok(());
    }

    if stored_market == Felt::ZERO {
        create_perpetual_balance(ctx, state, contract, account, market, balance, mode)?;
        return Ok(());
    }

    ctx.storage_write(contract, base, balance.market);
    ctx.storage_write(contract, storage_key_with_offset(base, 1), balance.amount);
    ctx.storage_write(contract, storage_key_with_offset(base, 2), balance.cost);
    ctx.storage_write(contract, storage_key_with_offset(base, 3), balance.cached_funding);
    ctx.storage_write(contract, storage_key_with_offset(base, 4), balance.prev);
    ctx.storage_write(contract, storage_key_with_offset(base, 5), balance.next);
    Ok(())
}

fn ensure_perpetual_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    mode: PerpStorageMode,
) -> Result<(), ExecutionError> {
    let base = resolve_perp_balance_base(state, ctx, contract, account, market)?;
    let stored_market = ctx.storage_read(state, contract, base)?;
    if stored_market == Felt::ZERO {
        let empty = PerpetualBalance {
            market,
            amount: Felt::ZERO,
            cost: Felt::ZERO,
            cached_funding: Felt::ZERO,
            prev: Felt::ZERO,
            next: Felt::ZERO,
        };
        create_perpetual_balance(ctx, state, contract, account, market, &empty, mode)?;
    }
    Ok(())
}

fn create_perpetual_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    balance: &PerpetualBalance,
    mode: PerpStorageMode,
) -> Result<(), ExecutionError> {
    let tail_key = perp_balance_tail_key(mode, account);
    let tail = ctx.storage_read(state, contract, tail_key)?;
    let _ = mode;
    let base = resolve_perp_balance_base(state, ctx, contract, account, market)?;
    ctx.storage_write(contract, base, market);
    ctx.storage_write(contract, storage_key_with_offset(base, 1), balance.amount);
    ctx.storage_write(contract, storage_key_with_offset(base, 2), balance.cost);
    ctx.storage_write(contract, storage_key_with_offset(base, 3), balance.cached_funding);
    ctx.storage_write(contract, storage_key_with_offset(base, 4), tail);
    ctx.storage_write(contract, storage_key_with_offset(base, 5), Felt::ZERO);
    ctx.storage_write(contract, tail_key, market);
    if tail != Felt::ZERO {
        let tail_base = resolve_perp_balance_base(state, ctx, contract, account, tail)?;
        ctx.storage_write(contract, storage_key_with_offset(tail_base, 5), market);
    }
    Ok(())
}

fn remove_perpetual_balance<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    market: Felt,
    mode: PerpStorageMode,
) -> Result<(), ExecutionError> {
    let _ = mode;
    let base = resolve_perp_balance_base(state, ctx, contract, account, market)?;
    let prev = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    let next = ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?;
    if prev != Felt::ZERO {
        let prev_base = resolve_perp_balance_base(state, ctx, contract, account, prev)?;
        ctx.storage_write(contract, storage_key_with_offset(prev_base, 5), next);
    }
    if next != Felt::ZERO {
        let next_base = resolve_perp_balance_base(state, ctx, contract, account, next)?;
        ctx.storage_write(contract, storage_key_with_offset(next_base, 4), prev);
    }
    let tail_key = perp_balance_tail_key(mode, account);
    let tail = ctx.storage_read(state, contract, tail_key)?;
    if tail == market {
        ctx.storage_write(contract, tail_key, prev);
    }
    ctx.storage_write(contract, base, Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 1), Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 2), Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 3), Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 4), Felt::ZERO);
    ctx.storage_write(contract, storage_key_with_offset(base, 5), Felt::ZERO);
    Ok(())
}

fn upsert_total_realized_pnl_and_funding<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    account: ContractAddress,
    realized_pnl: i128,
    realized_funding: i128,
    mode: PerpStorageMode,
) -> Result<(), ExecutionError> {
    let base = perp_invariants_info_key(mode, market, account);
    let pnl_key = storage_key_with_offset(base, 0);
    let funding_key = storage_key_with_offset(base, 1);
    let current_pnl = felt_to_i128(ctx.storage_read(state, contract, pnl_key)?)?;
    let current_funding = felt_to_i128(ctx.storage_read(state, contract, funding_key)?)?;
    ctx.storage_write(contract, pnl_key, i128_to_felt(current_pnl + realized_pnl));
    ctx.storage_write(contract, funding_key, i128_to_felt(current_funding + realized_funding));
    Ok(())
}

fn position_after_trade(
    balance: &PerpetualBalance,
    trade_price: i128,
    trade_size: i128,
    order_side: Felt,
    current_funding: i128,
    settlement_token_price: i128,
) -> Result<(PerpetualBalance, i128, i128), ExecutionError> {
    let market = balance.market;
    if market == Felt::ZERO {
        return Err(ExecutionError::ExecutionFailed("position should be initialized".to_string()));
    }
    let cost_price = div_128_scaled(trade_price, settlement_token_price)?;
    let (updated_amount, updated_cost, rpnl) =
        calculate_amount_cost(market, order_side, trade_size, cost_price, balance)?;
    let (rpnl_funding, new_funding_index) =
        calculate_funding_delta_and_new_index(balance, trade_size, order_side, current_funding)?;
    let updated = PerpetualBalance {
        market,
        amount: i128_to_felt(updated_amount),
        cost: i128_to_felt(updated_cost),
        cached_funding: i128_to_felt(new_funding_index),
        prev: balance.prev,
        next: balance.next,
    };
    Ok((updated, rpnl_funding, rpnl))
}

fn calculate_amount_cost(
    _market: Felt,
    order_side: Felt,
    trade_size: i128,
    cost_price: i128,
    previous_balance: &PerpetualBalance,
) -> Result<(i128, i128, i128), ExecutionError> {
    let order_sign = get_sign_from_side(order_side)?;
    let trade_amount = order_sign * trade_size;
    let trade_value = mul_128_scaled(trade_amount, cost_price)?;

    if previous_balance.amount == Felt::ZERO {
        return Ok((trade_amount, trade_value, 0));
    }

    let previous_amount = felt_to_i128(previous_balance.amount)?;
    let previous_cost = felt_to_i128(previous_balance.cost)?;
    let updated_amount = previous_amount + trade_amount;
    let previous_balance_side = get_side_from_sign(previous_amount);
    if previous_balance_side == order_side {
        let updated_cost = previous_cost + trade_value;
        let rpnl = calculate_realized_pnl(previous_cost, updated_cost, trade_value);
        return Ok((updated_amount, updated_cost, rpnl));
    }

    let previous_amount_abs = abs_128(previous_amount);
    if trade_size <= previous_amount_abs {
        let size_ratio = div_128_scaled(updated_amount, previous_amount)?;
        let updated_cost = mul_128_scaled(previous_cost, size_ratio)?;
        let rpnl = calculate_realized_pnl(previous_cost, updated_cost, trade_value);
        Ok((updated_amount, updated_cost, rpnl))
    } else {
        let updated_cost = mul_128_scaled(cost_price, updated_amount)?;
        let rpnl = calculate_realized_pnl(previous_cost, updated_cost, trade_value);
        Ok((updated_amount, updated_cost, rpnl))
    }
}

fn calculate_funding_delta_and_new_index(
    balance: &PerpetualBalance,
    trade_size: i128,
    side: Felt,
    market_funding_index: i128,
) -> Result<(i128, i128), ExecutionError> {
    if balance.amount == Felt::ZERO {
        return Ok((0, market_funding_index));
    }
    let previous_funding_index = felt_to_i128(balance.cached_funding)?;
    if trade_size == 0 {
        return Ok((0, previous_funding_index));
    }
    let current_amount = felt_to_i128(balance.amount)?;
    let current_balance_side = get_side_from_sign(current_amount);
    let trade_size_abs = abs_128(trade_size);
    let current_amount_abs = abs_128(current_amount);

    if current_balance_side == side {
        let total_size = current_amount_abs + trade_size_abs;
        let existing_weighted = mul_128_scaled(current_amount_abs, previous_funding_index)?;
        let new_weighted = mul_128_scaled(trade_size_abs, market_funding_index)?;
        let total_weighted = existing_weighted + new_weighted;
        return Ok((0, div_128_scaled(total_weighted, total_size)?));
    }

    let funding_delta = market_funding_index - previous_funding_index;
    let realized_size = min_128(current_amount_abs, trade_size_abs);
    let mut funding_delta_scaled = mul_128_scaled(funding_delta, realized_size)?;
    if current_balance_side == buy_side() {
        funding_delta_scaled = -funding_delta_scaled;
    }
    if trade_size_abs <= current_amount_abs {
        Ok((funding_delta_scaled, previous_funding_index))
    } else {
        Ok((funding_delta_scaled, market_funding_index))
    }
}

fn reduce_only_will_increase(
    balance: &PerpetualBalance,
    order_size: i128,
    order_side: Felt,
) -> Result<bool, ExecutionError> {
    if balance.amount == Felt::ZERO {
        return Ok(true);
    }
    let position_size = felt_to_i128(balance.amount)?;
    let order_sign = get_sign_from_side(order_side)?;
    Ok(position_size * order_sign > 0 || order_size > abs_128(position_size))
}

fn get_sign_from_side(side: Felt) -> Result<i128, ExecutionError> {
    if side == buy_side() {
        Ok(1)
    } else if side == sell_side() {
        Ok(-1)
    } else {
        Err(ExecutionError::ExecutionFailed("invalid side".to_string()))
    }
}

fn get_side_from_sign(amount: i128) -> Felt {
    if amount >= 0 {
        buy_side()
    } else {
        sell_side()
    }
}

fn calculate_realized_pnl(old_cost: i128, new_cost: i128, trade_value: i128) -> i128 {
    new_cost - old_cost - trade_value
}

fn abs_128(value: i128) -> i128 {
    if value >= 0 {
        value
    } else {
        -value
    }
}

fn min_128(a: i128, b: i128) -> i128 {
    if a <= b {
        a
    } else {
        b
    }
}

fn max_128(a: i128, b: i128) -> i128 {
    if a >= b {
        a
    } else {
        b
    }
}

fn felt_to_i128(value: Felt) -> Result<i128, ExecutionError> {
    // Cairo i128 is stored as a felt in [-2^127, 2^127-1].
    // Negative values are represented as PRIME - |value|, and PRIME mod 2^128 == 1,
    // so a naive low-128 conversion yields -(x-1). Adjust by -1 for negatives.
    const MAX_I128_U128: u128 = u128::MAX >> 1; // 2^127 - 1
    let max_i128_felt = Felt::from(MAX_I128_U128);
    let bytes = value.to_bytes_be();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    let mut out = i128::from_be_bytes(arr);
    if value > max_i128_felt {
        out -= 1;
    }
    Ok(out)
}

fn i128_to_felt(value: i128) -> Felt {
    if value >= 0 {
        Felt::from(value as u128)
    } else {
        Felt::ZERO - Felt::from((-value) as u128)
    }
}

fn is_risky_after_trade_from_state<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    _account: ContractAddress,
    settlement_token_address: ContractAddress,
    overrides: RiskOverrides,
    trade_fee: i128,
    mut account_state: AccountStateLite,
) -> Result<bool, ExecutionError> {
    // (debug logs removed)

    if let Some((token_address, _before, after)) = overrides.spot_token {
        let mut found = false;
        for (idx, balance) in account_state.token_balances.iter_mut().enumerate() {
            if balance.token_address == token_address {
                balance.amount = after;
                if token_address == settlement_token_address {
                    account_state.settlement_token_index = idx;
                }
                found = true;
                break;
            }
        }
        if !found {
            let token_name = read_token_name(ctx, state, contract, token_address)?;
            let oracle_address =
                read_contract_address(ctx, state, contract, *layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE)?;
            let price_i128 = read_perp_price_i128(ctx, state, oracle_address, token_name)?;
            account_state.token_balances.push(TokenBalanceLite { token_address, token_name, amount: after });
            account_state.token_prices.push(price_i128);
            if token_address == settlement_token_address {
                account_state.settlement_token_index = account_state.token_balances.len() - 1;
            }
        }
    }

    if let Some((market, _before, after, cost, cached_funding)) = overrides.perp_market {
        let mut found = false;
        for balance in account_state.perp_balances.iter_mut() {
            if balance.market == market {
                balance.amount = i128_to_felt(after);
                balance.cost = cost;
                balance.cached_funding = cached_funding;
                found = true;
                break;
            }
        }
        if !found {
            let oracle_address =
                read_contract_address(ctx, state, contract, *layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE)?;
            let assets_manager_address = read_contract_address(ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;
            let price_i128 = read_perp_price_i128(ctx, state, oracle_address, market)?;
            let funding_i128 = read_funding_index_i128(ctx, state, oracle_address, market)?;
            let mode = resolve_perp_storage_mode(ctx, state, contract, market)?;
            let asset_kind = get_asset_kind(ctx, state, contract, assets_manager_address, market)?;
            account_state.perp_balances.push(PerpetualBalance {
                market,
                amount: i128_to_felt(after),
                cost,
                cached_funding,
                prev: Felt::ZERO,
                next: Felt::ZERO,
            });
            account_state.perp_prices.push(price_i128);
            account_state.perp_funding_indices.push(funding_i128);
            account_state.perp_asset_kinds.push(asset_kind);
            if asset_kind == assets_manager::asset_kind_future() {
                let asset_key = perp_asset_key(mode, market);
                let imf = read_perpetual_imf_base_with_key(ctx, state, contract, asset_key)?;
                let fee = FeeWithCapLite {
                    fee: assets_manager::get_market_fee_provision_rate(
                        ctx,
                        state,
                        assets_manager_address,
                        assets_manager::asset_kind_future(),
                    )?,
                    fee_cap: 0,
                    fee_floor: 0,
                };
                account_state.perp_imf_base.push(imf);
                account_state.perp_option_assets.push(None);
                account_state.perp_fee_provision_rate.push(fee);
            } else {
                let option_asset = assets_manager::get_option_asset(ctx, state, assets_manager_address, market)?;
                let option_asset_lite = OptionAssetLite {
                    base_asset: option_asset.base_asset,
                    option_type: option_asset.option_type,
                    strike: felt_to_i128(option_asset.strike)?,
                };
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    account_state.spot_prices.entry(option_asset.base_asset)
                {
                    entry.insert(read_perp_price_i128(ctx, state, oracle_address, option_asset.base_asset)?);
                }
                let cache_key = (asset_kind, option_asset.base_asset);
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    account_state.option_margin_params.entry(cache_key)
                {
                    let params = assets_manager::get_option_margin_params(
                        ctx,
                        state,
                        assets_manager_address,
                        asset_kind,
                        option_asset.base_asset,
                    )?;
                    entry.insert(option_cross_margin_params_lite_from_assets(params)?);
                }
                let fee = fee_with_cap_lite_from_assets(assets_manager::get_option_fee_provision_rate(
                    ctx,
                    state,
                    assets_manager_address,
                    asset_kind,
                )?)?;
                account_state.perp_imf_base.push(0);
                account_state.perp_option_assets.push(Some(option_asset_lite));
                account_state.perp_fee_provision_rate.push(fee);
            }
        }
    }

    if let Some(settlement_after) = overrides.settlement_token_amount {
        if let Some(balance) =
            account_state.token_balances.iter_mut().find(|b| b.token_address == settlement_token_address)
        {
            balance.amount = settlement_after;
        }
    }

    let (margin_requirement, total_upnl) = margin_requirement_and_total_upnl(&account_state, true)?;
    let asset_value = get_asset_value(&account_state)?;
    let account_value = asset_value + total_upnl;
    let free_balance = account_value - margin_requirement - trade_fee;
    if free_balance >= 0 {
        return Ok(false);
    }

    let (before, after) = if let Some((_, before, after)) = overrides.spot_token {
        (before, after)
    } else if let Some((_, before, after, _, _)) = overrides.perp_market {
        (before, after)
    } else {
        (0, 0)
    };
    let position_decrease = abs_128(before) - abs_128(after);
    if position_decrease >= 0 && account_value >= 0 {
        return Ok(false);
    }

    Ok(true)
}

fn is_risky_after_trade_loaded<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    settlement_token_address: ContractAddress,
    overrides: RiskOverrides,
    trade_fee: i128,
    account_state: AccountStateLite,
) -> Result<bool, ExecutionError> {
    time_inner(InnerTimingField::IsRiskyAfterTradeLoaded, || {
        is_risky_after_trade_from_state(
            ctx,
            state,
            contract,
            account,
            settlement_token_address,
            overrides,
            trade_fee,
            account_state,
        )
    })
}

#[allow(dead_code)]
fn is_risky_after_trade<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    settlement_token_address: ContractAddress,
    overrides: RiskOverrides,
    trade_fee: i128,
) -> Result<bool, ExecutionError> {
    let account_state = load_account_state_for_risk(ctx, state, contract, account, settlement_token_address)?;
    is_risky_after_trade_loaded(
        ctx,
        state,
        contract,
        account,
        settlement_token_address,
        overrides,
        trade_fee,
        account_state,
    )
}

fn load_account_state_for_risk<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    settlement_token_address: ContractAddress,
) -> Result<AccountStateLite, ExecutionError> {
    time_inner(InnerTimingField::LoadAccountStateForRisk, || {
        let referral = read_account_referral(ctx, state, contract, account)?;
        let margin_methodology = read_margin_methodology(ctx, state, contract, account)?;
        let fee_rates = read_fee_rates_for_account(ctx, state, contract, account)?;

        let assets_manager_address = read_contract_address(ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;
        let oracle_address =
            read_contract_address(ctx, state, contract, *layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE)?;
        let settlement_token_name = read_settlement_token_name(ctx, state, contract)?;
        let mut token_balances = load_token_balances(ctx, state, contract, account)?;
        let settlement_index =
            token_balances.iter().position(|b| b.token_name == settlement_token_name).unwrap_or(token_balances.len());
        if settlement_index == token_balances.len() {
            let settlement_amount = read_token_balance_amount(ctx, state, contract, account, settlement_token_address)?;
            token_balances.push(TokenBalanceLite {
                token_address: settlement_token_address,
                token_name: settlement_token_name,
                amount: settlement_amount,
            });
        }

        let mut token_prices = Vec::with_capacity(token_balances.len());
        for balance in &token_balances {
            let price_i128 = read_perp_price_i128(ctx, state, oracle_address, balance.token_name)?;
            token_prices.push(price_i128);
        }

        let settlement_token_price =
            felt_to_i128(oracle::read_settlement_token_price(ctx, state, oracle_address, oracle_settlement_key())?)?;

        let perp_mode = resolve_perp_storage_mode_for_account(ctx, state, contract, account)?;
        let perp_mmf_factor = felt_to_i128(ctx.storage_read(state, contract, perp_mmf_factor_key(perp_mode))?)?;

        let perp_state = load_perp_balances_and_prices(
            ctx,
            state,
            contract,
            account,
            perp_mode,
            oracle_address,
            assets_manager_address,
        )?;

        Ok(AccountStateLite {
            token_balances,
            token_prices,
            settlement_token_index: settlement_index,
            perp_balances: perp_state.balances,
            perp_prices: perp_state.prices,
            perp_funding_indices: perp_state.funding_indices,
            perp_imf_base: perp_state.imf_base,
            perp_option_assets: perp_state.option_assets,
            spot_prices: perp_state.spot_prices,
            option_margin_params: perp_state.option_margin_params,
            perp_fee_provision_rate: perp_state.fee_provision_rates,
            perp_asset_kinds: perp_state.asset_kinds,
            fee_rates,
            referral,
            margin_methodology,
            settlement_token_price,
            perp_mmf_factor,
        })
    })
}

fn load_token_balances<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<Vec<TokenBalanceLite>, ExecutionError> {
    time_inner(InnerTimingField::LoadTokenBalances, || {
        let mut balances = Vec::new();
        let inner = token_balance_inner_base(account);
        let tail_key = resolve_token_balance_tail_key(state, contract, account)?;
        let mut current = ctx.storage_read(state, contract, tail_key)?;
        while current != Felt::ZERO {
            let base = token_balance_base_from_inner(inner, ContractAddress(current));
            let amount = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?)?;
            let token_name = read_token_name(ctx, state, contract, ContractAddress(current))?;
            balances.push(TokenBalanceLite { token_address: ContractAddress(current), token_name, amount });
            current = ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?;
        }
        Ok(balances)
    })
}

fn load_perp_balances_and_prices<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    mode: PerpStorageMode,
    oracle_address: ContractAddress,
    assets_manager_address: ContractAddress,
) -> Result<LoadedPerpStateLite, ExecutionError> {
    time_inner(InnerTimingField::LoadPerpBalancesAndPrices, || {
        let mut loaded = LoadedPerpStateLite::default();
        let mut perp_asset_keys: HashMap<Felt, StorageKey> = HashMap::new();
        let mut future_provision_rate: Option<FeeWithCapLite> = None;
        let mut perp_option_provision_rate: Option<FeeWithCapLite> = None;
        let mut dated_option_provision_rate: Option<FeeWithCapLite> = None;

        let tail_key = perp_balance_tail_key(mode, account);
        let inner =
            if matches!(mode, PerpStorageMode::LegacyPedersen) { Some(perp_balance_inner_base(account)) } else { None };
        let mut current = ctx.storage_read(state, contract, tail_key)?;
        while current != Felt::ZERO {
            let balance = if let Some(inner) = inner {
                read_perpetual_balance_with_inner(ctx, state, contract, inner, current)?
            } else {
                read_perpetual_balance(ctx, state, contract, account, current, mode)?
            };
            loaded.balances.push(balance.clone());

            let price_i128 = read_perp_price_i128(ctx, state, oracle_address, current)?;
            loaded.prices.push(price_i128);
            let funding_i128 = read_funding_index_i128(ctx, state, oracle_address, current)?;
            loaded.funding_indices.push(funding_i128);

            let asset_key = *perp_asset_keys.entry(current).or_insert_with(|| perp_asset_key(mode, current));
            let asset_kind = get_asset_kind(ctx, state, contract, assets_manager_address, current)?;
            loaded.asset_kinds.push(asset_kind);

            if asset_kind == assets_manager::asset_kind_future() {
                let imf = read_perpetual_imf_base_with_key(ctx, state, contract, asset_key)?;
                loaded.imf_base.push(imf);
                loaded.option_assets.push(None);
                let fee = match future_provision_rate {
                    Some(rate) => rate,
                    None => {
                        let rate = FeeWithCapLite {
                            fee: assets_manager::get_market_fee_provision_rate(
                                ctx,
                                state,
                                assets_manager_address,
                                assets_manager::asset_kind_future(),
                            )?,
                            fee_cap: 0,
                            fee_floor: 0,
                        };
                        future_provision_rate = Some(rate);
                        rate
                    }
                };
                loaded.fee_provision_rates.push(fee);
            } else {
                loaded.imf_base.push(0);
                let option_asset = assets_manager::get_option_asset(ctx, state, assets_manager_address, current)?;
                let option_asset_lite = OptionAssetLite {
                    base_asset: option_asset.base_asset,
                    option_type: option_asset.option_type,
                    strike: felt_to_i128(option_asset.strike)?,
                };
                loaded.option_assets.push(Some(option_asset_lite));

                if let std::collections::hash_map::Entry::Vacant(entry) =
                    loaded.spot_prices.entry(option_asset.base_asset)
                {
                    entry.insert(read_perp_price_i128(ctx, state, oracle_address, option_asset.base_asset)?);
                }

                let cache_key = (asset_kind, option_asset.base_asset);
                if let std::collections::hash_map::Entry::Vacant(entry) = loaded.option_margin_params.entry(cache_key) {
                    let params = assets_manager::get_option_margin_params(
                        ctx,
                        state,
                        assets_manager_address,
                        asset_kind,
                        option_asset.base_asset,
                    )?;
                    entry.insert(option_cross_margin_params_lite_from_assets(params)?);
                }

                let fee = if asset_kind == assets_manager::asset_kind_dated_option() {
                    match dated_option_provision_rate {
                        Some(rate) => rate,
                        None => {
                            let rate = fee_with_cap_lite_from_assets(assets_manager::get_option_fee_provision_rate(
                                ctx,
                                state,
                                assets_manager_address,
                                assets_manager::asset_kind_dated_option(),
                            )?)?;
                            dated_option_provision_rate = Some(rate);
                            rate
                        }
                    }
                } else {
                    match perp_option_provision_rate {
                        Some(rate) => rate,
                        None => {
                            let rate = fee_with_cap_lite_from_assets(assets_manager::get_option_fee_provision_rate(
                                ctx,
                                state,
                                assets_manager_address,
                                assets_manager::asset_kind_option(),
                            )?)?;
                            perp_option_provision_rate = Some(rate);
                            rate
                        }
                    }
                };
                loaded.fee_provision_rates.push(fee);
            }

            // Offset 4 is used as the list pointer in traversal; reuse the already-read value.
            current = balance.prev;
        }
        Ok(loaded)
    })
}

#[allow(dead_code)]
fn read_perpetual_imf_base<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    mode: PerpStorageMode,
) -> Result<i128, ExecutionError> {
    let base = perp_asset_key(mode, market);
    let imf_base = ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?;
    felt_to_i128(imf_base)
}

fn read_perpetual_imf_base_with_key<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    asset_key: StorageKey,
) -> Result<i128, ExecutionError> {
    let imf_base = ctx.storage_read(state, contract, storage_key_with_offset(asset_key, 4))?;
    felt_to_i128(imf_base)
}

#[allow(dead_code)]
fn read_fee_provision_rate<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    mode: PerpStorageMode,
) -> Result<Option<i128>, ExecutionError> {
    let base = perp_market_fee_config_v2_key(mode, market);
    let exists = ctx.storage_read(state, contract, base)?;
    if exists != Felt::ONE {
        return Ok(None);
    }
    let rate = ctx.storage_read(state, contract, perp_max_fee_rate_key(mode, market))?;
    Ok(Some(felt_to_i128(rate)?))
}

#[allow(dead_code)]
fn read_fee_provision_rate_with_keys<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    fee_config_key: StorageKey,
    max_fee_rate_key: StorageKey,
) -> Result<Option<i128>, ExecutionError> {
    let exists = ctx.storage_read(state, contract, fee_config_key)?;
    if exists != Felt::ONE {
        return Ok(None);
    }
    let rate = ctx.storage_read(state, contract, max_fee_rate_key)?;
    Ok(Some(felt_to_i128(rate)?))
}

fn read_token_name(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
    token_address: ContractAddress,
) -> Result<Felt, ExecutionError> {
    let base = token_layout::Paraclear_token_asset_key(token_address.0);
    Ok(ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?)
}

fn read_settlement_token_name(
    ctx: &mut ExecutionContext,
    state: &impl StateReader,
    contract: ContractAddress,
) -> Result<Felt, ExecutionError> {
    let base = *layout::PARACLEAR_SETTLEMENT_TOKEN_ASSET_BASE;
    Ok(ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?)
}

fn read_fee_rates_for_account<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<FeeRatesLite, ExecutionError> {
    let futures = read_fee_rate_struct(
        ctx,
        state,
        contract,
        account_layout::Paraclear_account_fee_rate_key(account.0),
        *account_layout::PARACLEAR_GLOBAL_FEE_RATE_BASE,
    )?;
    let options = read_fee_rate_struct(
        ctx,
        state,
        contract,
        storage_key_for_map("Paraclear_account_fee_rate_options", account.0),
        *account_layout::PARACLEAR_GLOBAL_FEE_RATE_OPTIONS_BASE,
    )?;
    let spot = read_fee_rate_struct(
        ctx,
        state,
        contract,
        storage_key_for_map("Paraclear_account_fee_rate_spot", account.0),
        *account_layout::PARACLEAR_GLOBAL_FEE_RATE_SPOT_BASE,
    )?;
    Ok(FeeRatesLite { futures, options, spot, settlement: SettlementFeeRatesLite::zero() })
}

fn read_fee_rate_struct<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account_base: StorageKey,
    global_base: StorageKey,
) -> Result<FeeRateLite, ExecutionError> {
    let account_exists = ctx.storage_read(state, contract, account_base)?;
    let base = if account_exists == Felt::ONE { account_base } else { global_base };
    let exists = if base == account_base { account_exists } else { ctx.storage_read(state, contract, base)? };
    let maker = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?)?;
    let taker = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?)?;
    Ok(FeeRateLite { exists: exists == Felt::ONE, maker, taker })
}

fn read_account_referral<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<AccountReferralLite, ExecutionError> {
    let base = account_layout::Paraclear_account_referral_key(account.0);
    let referrer = ContractAddress(ctx.storage_read(state, contract, base)?);
    let fee_commission = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?)?;
    let fee_discount = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?)?;
    Ok(AccountReferralLite { referrer, fee_commission, fee_discount })
}

fn read_margin_methodology<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
) -> Result<MarginMethodologyLite, ExecutionError> {
    let base = account_layout::account_margin_methodology_key(account.0);
    let value = ctx.storage_read(state, contract, base)?;
    if value == Felt::ZERO {
        Ok(MarginMethodologyLite::CrossMargin)
    } else {
        Ok(MarginMethodologyLite::PortfolioMargin)
    }
}

fn get_asset_value(state: &AccountStateLite) -> Result<i128, ExecutionError> {
    time_inner(InnerTimingField::GetAssetValue, || {
        let mut total = 0i128;
        for (idx, balance) in state.token_balances.iter().enumerate() {
            let price = state.token_prices[idx];
            total += mul_128_scaled(balance.amount, price)?;
        }
        Ok(total)
    })
}

fn margin_requirement_and_total_upnl(
    state: &AccountStateLite,
    include_tokens: bool,
) -> Result<(i128, i128), ExecutionError> {
    time_inner(InnerTimingField::MarginRequirementAndTotalUpnl, || {
        let mut margin_requirement = 0i128;
        let mut total_upnl = 0i128;

        match state.margin_methodology {
            MarginMethodologyLite::CrossMargin => {
                for i in 0..state.perp_balances.len() {
                    let balance = &state.perp_balances[i];
                    let mark_price = state.perp_prices[i];
                    let funding_index = state.perp_funding_indices[i];
                    let settlement_price = state.settlement_token_price;
                    total_upnl += unrealized_pnl(balance, mark_price, settlement_price)?;
                    total_upnl += unrealized_funding_pnl(balance, funding_index, settlement_price)?;

                    let margin = if state.perp_asset_kinds.get(i) == Some(&assets_manager::asset_kind_future()) {
                        let current_value_abs = abs_128(get_value(balance, mark_price)?);
                        let margin_fraction = state.perp_imf_base[i];
                        mul_128_scaled(current_value_abs, margin_fraction)?
                    } else {
                        let Some(option_asset) = state.perp_option_assets.get(i).and_then(|asset| *asset) else {
                            return Err(ExecutionError::ExecutionFailed("missing option asset data".to_string()));
                        };
                        let asset_kind = state.perp_asset_kinds[i];
                        let Some(spot_price) = state.spot_prices.get(&option_asset.base_asset).copied() else {
                            return Err(ExecutionError::ExecutionFailed("missing option spot price".to_string()));
                        };
                        let Some(params) =
                            state.option_margin_params.get(&(asset_kind, option_asset.base_asset)).copied()
                        else {
                            return Err(ExecutionError::ExecutionFailed("missing option margin params".to_string()));
                        };
                        option_margin_requirement(
                            felt_to_i128(balance.amount)?,
                            option_asset,
                            params.imf,
                            mark_price,
                            spot_price,
                        )?
                    };
                    margin_requirement += margin;

                    let abs_amount = abs_128(felt_to_i128(balance.amount)?);
                    let fee_config = state.perp_fee_provision_rate.get(i).copied().unwrap_or_default();
                    let base_fee = if state.perp_asset_kinds.get(i) == Some(&assets_manager::asset_kind_future()) {
                        mul3_128_scaled(abs_amount, mark_price, fee_config.fee)?
                    } else {
                        let Some(option_asset) = state.perp_option_assets.get(i).and_then(|asset| *asset) else {
                            return Err(ExecutionError::ExecutionFailed("missing option asset data".to_string()));
                        };
                        let Some(spot_price) = state.spot_prices.get(&option_asset.base_asset).copied() else {
                            return Err(ExecutionError::ExecutionFailed("missing option spot price".to_string()));
                        };
                        option_calculate_fee(option_asset, abs_amount, mark_price, spot_price, fee_config)?
                    };
                    margin_requirement += base_fee;
                }
            }
            MarginMethodologyLite::PortfolioMargin => {
                for i in 0..state.perp_balances.len() {
                    let balance = &state.perp_balances[i];
                    let mark_price = state.perp_prices[i];
                    let funding_index = state.perp_funding_indices[i];
                    let settlement_price = state.settlement_token_price;
                    total_upnl += unrealized_pnl(balance, mark_price, settlement_price)?;
                    total_upnl += unrealized_funding_pnl(balance, funding_index, settlement_price)?;
                }
            }
        }

        if include_tokens {
            for (idx, balance) in state.token_balances.iter().enumerate() {
                if idx == state.settlement_token_index {
                    continue;
                }
                let price = state.token_prices[idx];
                margin_requirement += mul_128_scaled(balance.amount, price)?;
            }
        }

        Ok((margin_requirement, total_upnl))
    })
}

#[allow(dead_code)]
fn apply_referral_discount(fee: i128, referral: &AccountReferralLite) -> Result<i128, ExecutionError> {
    if fee < 0 {
        return Ok(fee);
    }
    if referral.referrer.0 == Felt::ZERO {
        return Ok(fee);
    }
    if referral.fee_discount != 0 {
        let one_minus = MULTIPLIER - referral.fee_discount;
        return mul_128_scaled(fee, one_minus);
    }
    Ok(fee)
}

fn get_value(balance: &PerpetualBalance, mark_price: i128) -> Result<i128, ExecutionError> {
    let amount = felt_to_i128(balance.amount)?;
    mul_128_scaled(amount, mark_price)
}

fn unrealized_pnl(
    balance: &PerpetualBalance,
    mark_price: i128,
    settlement_token_price: i128,
) -> Result<i128, ExecutionError> {
    let current_value = mul_128_scaled(felt_to_i128(balance.amount)?, mark_price)?;
    let previous_value = mul_128_scaled(felt_to_i128(balance.cost)?, settlement_token_price)?;
    Ok(current_value - previous_value)
}

fn unrealized_funding_pnl(
    balance: &PerpetualBalance,
    funding_index: i128,
    settlement_token_price: i128,
) -> Result<i128, ExecutionError> {
    let cached_funding = felt_to_i128(balance.cached_funding)?;
    let funding_diff = cached_funding - funding_index;
    let amount = felt_to_i128(balance.amount)?;
    let funding_in_settlement = mul_128_scaled(funding_diff, amount)?;
    mul_128_scaled(funding_in_settlement, settlement_token_price)
}

const MULTIPLIER: i128 = 100_000_000;
const MULTIPLIER_2: i128 = MULTIPLIER * MULTIPLIER;

fn mul_128_scaled(a: i128, b: i128) -> Result<i128, ExecutionError> {
    if a == 0 || b == 0 {
        return Ok(0);
    }
    let prod = a.checked_mul(b).ok_or_else(|| ExecutionError::ExecutionFailed("i128 overflow".to_string()))?;
    Ok(prod / MULTIPLIER)
}

fn mul3_128_scaled(a: i128, b: i128, c: i128) -> Result<i128, ExecutionError> {
    if a == 0 || b == 0 || c == 0 {
        return Ok(0);
    }
    let prod = a
        .checked_mul(b)
        .and_then(|v| v.checked_mul(c))
        .ok_or_else(|| ExecutionError::ExecutionFailed("i128 overflow".to_string()))?;
    Ok(prod / MULTIPLIER_2)
}

fn div_128_scaled(a: i128, b: i128) -> Result<i128, ExecutionError> {
    if b == 0 {
        return Err(ExecutionError::ExecutionFailed("division by zero".to_string()));
    }
    if a == 0 {
        return Ok(0);
    }
    let prod = a.checked_mul(MULTIPLIER).ok_or_else(|| ExecutionError::ExecutionFailed("i128 overflow".to_string()))?;
    Ok(prod / b)
}

fn option_margin_fraction(
    position_amount: i128,
    asset: OptionAssetLite,
    params: OptionMarginParamsLite,
    mark_price: i128,
    spot_price: i128,
) -> Result<i128, ExecutionError> {
    if position_amount >= 0 {
        return Ok(min_128(
            mul_128_scaled(params.premium_multiplier, mark_price)?,
            mul_128_scaled(params.long_itm, spot_price)?,
        ));
    }

    let otm_amount = if is_put_option(asset.option_type) {
        max_128(0, spot_price - asset.strike)
    } else {
        max_128(0, asset.strike - spot_price)
    };
    let margin = max_128(
        mul_128_scaled(params.short_itm, spot_price)? - otm_amount,
        mul_128_scaled(params.short_otm, spot_price)?,
    );
    if is_put_option(asset.option_type) {
        Ok(min_128(margin, mul_128_scaled(params.short_put_cap, asset.strike)?))
    } else {
        Ok(margin)
    }
}

fn option_margin_requirement(
    position_amount: i128,
    asset: OptionAssetLite,
    params: OptionMarginParamsLite,
    mark_price: i128,
    spot_price: i128,
) -> Result<i128, ExecutionError> {
    let margin_fraction = option_margin_fraction(position_amount, asset, params, mark_price, spot_price)?;
    mul_128_scaled(margin_fraction, abs_128(position_amount))
}

fn option_calculate_fee(
    asset: OptionAssetLite,
    trade_size: i128,
    trade_price: i128,
    spot_price: i128,
    fee_with_cap: FeeWithCapLite,
) -> Result<i128, ExecutionError> {
    let base_fee = mul_128_scaled(spot_price, fee_with_cap.fee)?;
    let clamped = if base_fee > 0 {
        let fee_cap = mul_128_scaled(trade_price, fee_with_cap.fee_cap)?;
        min_128(base_fee, fee_cap)
    } else {
        let fee_floor = mul_128_scaled(trade_price, fee_with_cap.fee_floor)?;
        max_128(base_fee, fee_floor)
    };
    let _ = asset;
    mul_128_scaled(trade_size, clamped)
}

fn is_put_option(option_type: Felt) -> bool {
    option_type == Felt::from(2u64)
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct MarketFeeConfigLite {
    exists: bool,
    maker_api: i128,
    taker_api: i128,
    maker_rpi: i128,
    taker_rpi: i128,
    maker_interactive: i128,
    taker_interactive: i128,
}

#[allow(dead_code)]
fn trade_support_fee_v2(trade: &TradeRequestV3) -> bool {
    !matches!(trade.maker_order.order_category, OrderCategory::Unspecified)
        && !matches!(trade.taker_order.order_category, OrderCategory::Unspecified)
}

fn base_fee_spot<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
    is_maker: bool,
    fee_rates: &FeeRatesLite,
) -> Result<i128, ExecutionError> {
    let _ = fee_rates;
    let trade_size = felt_to_i128(trade.size)?;
    let trade_price = felt_to_i128(trade.price)?;
    let order_category = if is_maker { &trade.maker_order.order_category } else { &trade.taker_order.order_category };
    let fee_rate = match order_category {
        OrderCategory::Dynamic(fee) => felt_to_i128(fee.fee)?,
        OrderCategory::DynamicWithToken(fee) => felt_to_i128(fee.fee)?,
        OrderCategory::API | OrderCategory::RPI | OrderCategory::Interactive => {
            let assets_manager_address = read_contract_address(ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;
            assets_manager::get_market_fee_by_category(
                ctx,
                state,
                assets_manager_address,
                assets_manager::asset_kind_spot(),
                trade.maker_order.market,
                fee_category_for_order(order_category),
                is_maker,
            )?
        }
        OrderCategory::Unspecified => fee_rate_for_spot(fee_rates, is_maker),
    };
    mul3_128_scaled(trade_size, trade_price, fee_rate)
}

fn base_fee_perp<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    trade: &TradeRequestV3,
    is_maker: bool,
    mode: PerpStorageMode,
    asset_kind: Felt,
    fee_rates: &FeeRatesLite,
) -> Result<i128, ExecutionError> {
    let _ = mode;
    let _ = fee_rates;
    let trade_size = felt_to_i128(trade.size)?;
    let trade_price = felt_to_i128(trade.price)?;
    let order_category = if is_maker { &trade.maker_order.order_category } else { &trade.taker_order.order_category };
    let assets_manager_address = read_contract_address(ctx, state, contract, *layout::ASSETS_MANAGER_BASE)?;

    if asset_kind == assets_manager::asset_kind_future() {
        let fee_rate = match order_category {
            OrderCategory::Dynamic(fee) => felt_to_i128(fee.fee)?,
            OrderCategory::DynamicWithToken(fee) => felt_to_i128(fee.fee)?,
            OrderCategory::API | OrderCategory::RPI | OrderCategory::Interactive => {
                assets_manager::get_market_fee_by_category(
                    ctx,
                    state,
                    assets_manager_address,
                    asset_kind,
                    trade.maker_order.market,
                    fee_category_for_order(order_category),
                    is_maker,
                )?
            }
            OrderCategory::Unspecified => fee_rate_for_perp(fee_rates, is_maker, asset_kind),
        };
        return mul3_128_scaled(trade_size, trade_price, fee_rate);
    }

    let option_asset = assets_manager::get_option_asset(ctx, state, assets_manager_address, trade.maker_order.market)?;
    let oracle_address = read_contract_address(ctx, state, contract, *layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE)?;
    let spot_price = read_perp_price_i128(ctx, state, oracle_address, option_asset.base_asset)?;
    let fee_with_cap = match order_category {
        OrderCategory::Dynamic(fee) => FeeWithCapLite {
            fee: felt_to_i128(fee.fee)?,
            fee_cap: felt_to_i128(fee.fee_cap)?,
            fee_floor: felt_to_i128(fee.fee_floor)?,
        },
        OrderCategory::DynamicWithToken(fee) => FeeWithCapLite {
            fee: felt_to_i128(fee.fee)?,
            fee_cap: felt_to_i128(fee.fee_cap)?,
            fee_floor: felt_to_i128(fee.fee_floor)?,
        },
        OrderCategory::API | OrderCategory::RPI | OrderCategory::Interactive => {
            fee_with_cap_lite_from_assets(assets_manager::get_base_asset_option_fee_by_category(
                ctx,
                state,
                assets_manager_address,
                asset_kind,
                option_asset.base_asset,
                fee_category_for_order(order_category),
                is_maker,
            )?)?
        }
        OrderCategory::Unspecified => {
            FeeWithCapLite { fee: fee_rate_for_perp(fee_rates, is_maker, asset_kind), fee_cap: 0, fee_floor: 0 }
        }
    };
    option_calculate_fee(
        OptionAssetLite {
            base_asset: option_asset.base_asset,
            option_type: option_asset.option_type,
            strike: felt_to_i128(option_asset.strike)?,
        },
        trade_size,
        trade_price,
        spot_price,
        fee_with_cap,
    )
}

fn fee_rate_for_spot(fee_rates: &FeeRatesLite, is_maker: bool) -> i128 {
    if is_maker {
        fee_rates.spot.maker
    } else {
        fee_rates.spot.taker
    }
}

fn fee_rate_for_perp(fee_rates: &FeeRatesLite, is_maker: bool, asset_kind: Felt) -> i128 {
    let rate = if asset_kind == assets_manager::asset_kind_option()
        || asset_kind == assets_manager::asset_kind_dated_option()
    {
        fee_rates.options
    } else {
        fee_rates.futures
    };
    if is_maker {
        rate.maker
    } else {
        rate.taker
    }
}

fn fee_category_for_order(
    category: &OrderCategory,
) -> crate::contracts::paradex::schema::assets_manager_types::FeeCategory {
    match category {
        OrderCategory::API => crate::contracts::paradex::schema::assets_manager_types::FeeCategory::API,
        OrderCategory::RPI => crate::contracts::paradex::schema::assets_manager_types::FeeCategory::RPI,
        OrderCategory::Interactive => crate::contracts::paradex::schema::assets_manager_types::FeeCategory::Interactive,
        _ => crate::contracts::paradex::schema::assets_manager_types::FeeCategory::Unspecified,
    }
}

fn fee_with_cap_lite_from_assets(
    fee: crate::contracts::paradex::schema::assets_manager_types::FeeWithCap,
) -> Result<FeeWithCapLite, ExecutionError> {
    Ok(FeeWithCapLite {
        fee: felt_to_i128(fee.fee)?,
        fee_cap: felt_to_i128(fee.fee_cap)?,
        fee_floor: felt_to_i128(fee.fee_floor)?,
    })
}

fn option_margin_params_lite_from_assets(
    params: crate::contracts::paradex::schema::assets_manager_types::OptionMarginParams,
) -> Result<OptionMarginParamsLite, ExecutionError> {
    Ok(OptionMarginParamsLite {
        premium_multiplier: felt_to_i128(params.premium_multiplier)?,
        long_itm: felt_to_i128(params.long_itm)?,
        short_itm: felt_to_i128(params.short_itm)?,
        short_otm: felt_to_i128(params.short_otm)?,
        short_put_cap: felt_to_i128(params.short_put_cap)?,
    })
}

fn option_cross_margin_params_lite_from_assets(
    params: crate::contracts::paradex::schema::assets_manager_types::OptionCrossMarginParams,
) -> Result<OptionCrossMarginParamsLite, ExecutionError> {
    Ok(OptionCrossMarginParamsLite {
        imf: option_margin_params_lite_from_assets(params.imf)?,
        mmf: option_margin_params_lite_from_assets(params.mmf)?,
    })
}

fn fee_token_override(category: &OrderCategory, settlement_token_address: ContractAddress) -> Option<ContractAddress> {
    match category {
        OrderCategory::DynamicWithToken(req)
            if req.fee_token.0 != Felt::ZERO && req.fee_token != settlement_token_address =>
        {
            Some(req.fee_token)
        }
        _ => None,
    }
}

fn resolve_fee_token_address(
    maker_fee_token: Option<ContractAddress>,
    taker_fee_token: Option<ContractAddress>,
) -> Result<Option<ContractAddress>, ExecutionError> {
    match (maker_fee_token, taker_fee_token) {
        (Some(maker), Some(taker)) if maker != taker => {
            Err(ExecutionError::ExecutionFailed("mismatched fee token".to_string()))
        }
        (Some(token), _) | (_, Some(token)) => Ok(Some(token)),
        (None, None) => Ok(None),
    }
}

fn token_price_from_state(state: &AccountStateLite, token_address: ContractAddress) -> Option<i128> {
    state
        .token_balances
        .iter()
        .position(|balance| balance.token_address == token_address)
        .and_then(|idx| state.token_prices.get(idx).copied())
}

fn resolve_fee_token_price(
    fee_token_address: ContractAddress,
    maker_fee_token: Option<ContractAddress>,
    taker_fee_token: Option<ContractAddress>,
    maker_state: &AccountStateLite,
    taker_state: &AccountStateLite,
) -> Result<i128, ExecutionError> {
    let price = if maker_fee_token == Some(fee_token_address) {
        token_price_from_state(maker_state, fee_token_address)
    } else if taker_fee_token == Some(fee_token_address) {
        token_price_from_state(taker_state, fee_token_address)
    } else {
        None
    };
    price.ok_or_else(|| ExecutionError::ExecutionFailed("fee token not found in account balances".to_string()))
}

#[allow(dead_code)]
fn read_market_fee_config<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    mode: PerpStorageMode,
) -> Result<Option<MarketFeeConfigLite>, ExecutionError> {
    let base = perp_market_fee_config_v2_key(mode, market);
    let exists = ctx.storage_read(state, contract, base)?;
    if exists != Felt::ONE {
        return Ok(None);
    }
    let maker_api = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 1))?)?;
    let taker_api = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 2))?)?;
    let maker_rpi = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 3))?)?;
    let taker_rpi = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 4))?)?;
    let maker_interactive = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 5))?)?;
    let taker_interactive = felt_to_i128(ctx.storage_read(state, contract, storage_key_with_offset(base, 6))?)?;
    Ok(Some(MarketFeeConfigLite {
        exists: true,
        maker_api,
        taker_api,
        maker_rpi,
        taker_rpi,
        maker_interactive,
        taker_interactive,
    }))
}

#[allow(dead_code)]
fn fee_rate_v2_for_order<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    market: Felt,
    category: &OrderCategory,
    is_maker: bool,
    mode: PerpStorageMode,
) -> Result<Option<i128>, ExecutionError> {
    let config = read_market_fee_config(ctx, state, contract, market, mode)?;
    let Some(cfg) = config else {
        return Ok(None);
    };
    if let OrderCategory::Dynamic(fee) = category {
        return Ok(Some(felt_to_i128(fee.fee)?));
    }
    let rate = match category {
        OrderCategory::API => {
            if is_maker {
                cfg.maker_api
            } else {
                cfg.taker_api
            }
        }
        OrderCategory::RPI => {
            if is_maker {
                cfg.maker_rpi
            } else {
                cfg.taker_rpi
            }
        }
        OrderCategory::Interactive => {
            if is_maker {
                cfg.maker_interactive
            } else {
                cfg.taker_interactive
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(rate))
}

#[allow(dead_code)]
fn get_trade_fee_and_referral_commission(
    fee: i128,
    referral: AccountReferralLite,
) -> Result<(i128, ContractAddress, i128), ExecutionError> {
    if fee < 0 {
        return Ok((fee, ContractAddress(Felt::ZERO), 0));
    }
    if referral.referrer.0 == Felt::ZERO {
        return Ok((fee, ContractAddress(Felt::ZERO), 0));
    }
    if referral.fee_discount != 0 {
        let one_minus_discount = MULTIPLIER - referral.fee_discount;
        let fee_after_discount = mul_128_scaled(fee, one_minus_discount)?;
        if referral.fee_commission != 0 {
            let fee_commission = mul_128_scaled(fee_after_discount, referral.fee_commission)?;
            return Ok((fee_after_discount, referral.referrer, fee_commission));
        }
        return Ok((fee_after_discount, referral.referrer, 0));
    }
    if referral.fee_commission != 0 {
        let fee_commission = mul_128_scaled(fee, referral.fee_commission)?;
        return Ok((fee, referral.referrer, fee_commission));
    }
    Ok((fee, referral.referrer, 0))
}

fn read_max_pending_transfer_executions<S: StateReader>(
    state: &S,
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
) -> Result<u64, ExecutionError> {
    let base = *layout::GLOBAL_CONFIGURATION_BASE;
    let value = ctx.storage_read(state, contract, base)?;
    felt_to_u64(value)
}

fn add_pending_transfer<S: StateReader>(
    state: &S,
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    executor: ContractAddress,
    trade_id: Felt,
    recipient: ContractAddress,
    token_address: ContractAddress,
    amount: i128,
) -> Result<(), ExecutionError> {
    if amount == 0 {
        return Ok(());
    }
    let count_key = layout::pending_transfer_count_by_executor_key(executor.0);
    let current = felt_to_u64(ctx.storage_read(state, contract, count_key)?)?;
    let next = current + 1;
    if let Some(diagnostic) = crate::telemetry::tx_diff::current() {
        tracing::info!(
            target: "RUST_EXEC",
            "pending_transfer_stage block_number={} tx_hash={:#x} stage=count_updated contract_address={:#x} executor={:#x} trade_id={:#x} recipient={:#x} token_address={:#x} amount={} count_key={:#x} previous_count={} next_count={}",
            diagnostic.block_number,
            diagnostic.tx_hash,
            contract.0,
            executor.0,
            trade_id,
            recipient.0,
            token_address.0,
            amount,
            count_key.0,
            current,
            next,
        );
    }
    ctx.storage_write(contract, count_key, Felt::from(next));

    let base = layout::pending_transfers_key2(executor.0, Felt::from(next));
    for (offset, value) in [(0, trade_id), (1, recipient.0), (2, token_address.0), (3, i128_to_felt(amount))] {
        let key = storage_key_with_offset(base, offset);
        if let Some(diagnostic) = crate::telemetry::tx_diff::current() {
            tracing::info!(
                target: "RUST_EXEC",
                "pending_transfer_stage block_number={} tx_hash={:#x} stage=field_constructed contract_address={:#x} executor={:#x} trade_id={:#x} transfer_index={} field_offset={} storage_key={:#x} value={:#x} is_zero={}",
                diagnostic.block_number,
                diagnostic.tx_hash,
                contract.0,
                executor.0,
                trade_id,
                next,
                offset,
                key.0,
                value,
                value == Felt::ZERO,
            );
        }
        ctx.storage_write_explicit(contract, key, value);
    }
    Ok(())
}

fn fee_payments<S: StateReader>(
    ctx: &mut ExecutionContext,
    state: &S,
    contract: ContractAddress,
    caller: ContractAddress,
    trade_id: Felt,
    settlement_token_address: ContractAddress,
    fee_token_address: Option<ContractAddress>,
    fee_token_price: Option<i128>,
    maker: FeePaymentSide,
    taker: FeePaymentSide,
) -> Result<(i128, i128), ExecutionError> {
    let maker_after_fee =
        maker.settlement_after_trade - if maker.pays_in_fee_token { 0 } else { maker.fee_in_settlement };
    let taker_after_fee =
        taker.settlement_after_trade - if taker.pays_in_fee_token { 0 } else { taker.fee_in_settlement };

    if !maker.pays_in_fee_token {
        update_token_balance(ctx, state, contract, maker.account, settlement_token_address, -maker.fee_in_settlement)?;
    }
    if !taker.pays_in_fee_token {
        update_token_balance(ctx, state, contract, taker.account, settlement_token_address, -taker.fee_in_settlement)?;
    }

    let pending_transfers_enabled = read_max_pending_transfer_executions(state, ctx, contract)? > 0;

    let fee_share_account =
        ContractAddress(ctx.storage_read(state, contract, *layout::PARACLEAR_FEE_SHARE_ACCOUNT_ADDRESS_BASE)?);
    let fee_share_percentage =
        felt_to_i128(ctx.storage_read(state, contract, *layout::PARACLEAR_FEE_SHARE_PERCENTAGE_BASE)?)?;

    let total_fee = maker.fee_in_settlement + taker.fee_in_settlement;
    let mut fee_credited_to_account_fee = total_fee;
    let mut total_fee_share_in_settlement = 0;

    if fee_share_account.0 != Felt::ZERO && fee_share_percentage != 0 {
        let fee_share_amount = mul_128_scaled(total_fee, fee_share_percentage)?;
        fee_credited_to_account_fee -= fee_share_amount;
        total_fee_share_in_settlement = fee_share_amount;
    }

    if let Some(fee_token_address) = fee_token_address {
        let fee_token_price =
            fee_token_price.ok_or_else(|| ExecutionError::ExecutionFailed("missing fee token price".to_string()))?;
        let total_fee_token_in_settlement = (if maker.pays_in_fee_token { maker.fee_in_settlement } else { 0 })
            + (if taker.pays_in_fee_token { taker.fee_in_settlement } else { 0 });
        let fee_token_share_in_settlement = if fee_share_account.0 != Felt::ZERO && fee_share_percentage != 0 {
            mul_128_scaled(total_fee_token_in_settlement, fee_share_percentage)?
        } else {
            0
        };
        let fee_token_credit_in_settlement = total_fee_token_in_settlement - fee_token_share_in_settlement;
        fee_credited_to_account_fee -= fee_token_credit_in_settlement;
        total_fee_share_in_settlement -= fee_token_share_in_settlement;

        let fee_token_credit_in_fee_token = div_128_scaled(fee_token_credit_in_settlement, fee_token_price)?;
        let fee_token_share_in_fee_token = div_128_scaled(fee_token_share_in_settlement, fee_token_price)?;

        if maker.fee_token_deduction != 0 {
            update_token_balance(ctx, state, contract, maker.account, fee_token_address, -maker.fee_token_deduction)?;
        }
        if taker.fee_token_deduction != 0 {
            update_token_balance(ctx, state, contract, taker.account, fee_token_address, -taker.fee_token_deduction)?;
        }

        if fee_token_credit_in_fee_token != 0 {
            let fee_account =
                ContractAddress(ctx.storage_read(state, contract, *layout::PARACLEAR_FEE_ACCOUNT_ADDRESS_BASE)?);
            if pending_transfers_enabled {
                add_pending_transfer(
                    state,
                    ctx,
                    contract,
                    caller,
                    trade_id,
                    fee_account,
                    fee_token_address,
                    fee_token_credit_in_fee_token,
                )?;
            } else {
                update_token_balance(
                    ctx,
                    state,
                    contract,
                    fee_account,
                    fee_token_address,
                    fee_token_credit_in_fee_token,
                )?;
            }
        }

        if fee_token_share_in_fee_token != 0 {
            if pending_transfers_enabled {
                add_pending_transfer(
                    state,
                    ctx,
                    contract,
                    caller,
                    trade_id,
                    fee_share_account,
                    fee_token_address,
                    fee_token_share_in_fee_token,
                )?;
            } else {
                update_token_balance(
                    ctx,
                    state,
                    contract,
                    fee_share_account,
                    fee_token_address,
                    fee_token_share_in_fee_token,
                )?;
            }
        }

        if fee_token_share_in_fee_token != 0 {
            emit_fee_share_v2(ctx, fee_share_account, fee_token_share_in_fee_token, fee_token_address);
        }
    }

    if total_fee_share_in_settlement != 0 {
        if pending_transfers_enabled {
            add_pending_transfer(
                state,
                ctx,
                contract,
                caller,
                trade_id,
                fee_share_account,
                settlement_token_address,
                total_fee_share_in_settlement,
            )?;
        } else {
            update_token_balance(
                ctx,
                state,
                contract,
                fee_share_account,
                settlement_token_address,
                total_fee_share_in_settlement,
            )?;
        }
        emit_fee_share_v2(ctx, fee_share_account, total_fee_share_in_settlement, settlement_token_address);
    }

    if fee_credited_to_account_fee < 0 {
        return Err(ExecutionError::ExecutionFailed("Fee credited to fee account is negative".to_string()));
    }

    let fee_account =
        ContractAddress(ctx.storage_read(state, contract, *layout::PARACLEAR_FEE_ACCOUNT_ADDRESS_BASE)?);
    if pending_transfers_enabled {
        add_pending_transfer(
            state,
            ctx,
            contract,
            caller,
            trade_id,
            fee_account,
            settlement_token_address,
            fee_credited_to_account_fee,
        )?;
    } else {
        update_token_balance(ctx, state, contract, fee_account, settlement_token_address, fee_credited_to_account_fee)?;
    }

    emit_fee_v2(
        ctx,
        maker.account,
        if maker.pays_in_fee_token { maker.fee_token_deduction } else { maker.fee_in_settlement },
        maker.payment_token_address,
    );
    emit_fee_v2(
        ctx,
        taker.account,
        if taker.pays_in_fee_token { taker.fee_token_deduction } else { taker.fee_in_settlement },
        taker.payment_token_address,
    );

    Ok((maker_after_fee, taker_after_fee))
}

fn emit_token_balance_update(
    ctx: &mut ExecutionContext,
    account: ContractAddress,
    token_address: ContractAddress,
    prev_amount: Felt,
    updated_amount: Felt,
    is_liquidation: Felt,
) {
    let selector = event_selector("TokenAssetBalanceUpdate");
    ctx.emit_event(vec![selector], vec![account.0, token_address.0, prev_amount, updated_amount, is_liquidation]);
}

fn emit_perpetual_balance_update(
    ctx: &mut ExecutionContext,
    account: ContractAddress,
    market: Felt,
    amount: Felt,
    cost: Felt,
    trade_id: Felt,
    trade_size: Felt,
    trade_price: Felt,
    realized_pnl: Felt,
    realized_funding: Felt,
    cached_funding: Felt,
    prev_funding: Felt,
    block_timestamp: Felt,
    settlement_token_price: Felt,
) {
    let selector = event_selector("PerpetualAssetBalanceUpdateV3");
    ctx.emit_event(
        vec![selector],
        vec![
            account.0,
            market,
            amount,
            cost,
            trade_id,
            trade_size,
            trade_price,
            realized_pnl,
            realized_funding,
            cached_funding,
            prev_funding,
            block_timestamp,
            settlement_token_price,
        ],
    );
}

fn emit_trade_settled(ctx: &mut ExecutionContext, trade_id: Felt, market: Felt, price: Felt, size: Felt) {
    let selector = event_selector("TradeSettled");
    ctx.emit_event(vec![selector], vec![trade_id, market, price, size]);
}

fn emit_fee_v2(ctx: &mut ExecutionContext, account: ContractAddress, fee: i128, fee_token_address: ContractAddress) {
    let selector = event_selector("FeeV2");
    ctx.emit_event(vec![selector], vec![account.0, i128_to_felt(fee), fee_token_address.0]);
}

fn emit_fee_share_v2(
    ctx: &mut ExecutionContext,
    account: ContractAddress,
    fee: i128,
    fee_token_address: ContractAddress,
) {
    let selector = event_selector("FeeShareV2");
    ctx.emit_event(vec![selector], vec![account.0, i128_to_felt(fee), fee_token_address.0]);
}

fn emit_settle_trade_failed(ctx: &mut ExecutionContext, error_code: Felt, error_message: Felt, trade: &TradeRequestV3) {
    let selector = event_selector("SettleTradeFailedV3");
    let mut data = Vec::new();
    data.push(error_code);
    data.push(error_message);
    data.extend(encode_trade_request_v3(trade));
    ctx.emit_event(vec![selector], data);
}

fn encode_trade_request_v3(trade: &TradeRequestV3) -> Vec<Felt> {
    let mut data = vec![trade.id, trade.size, trade.price, trade.traded_at];
    data.extend(encode_order_v3(&trade.maker_order));
    data.extend(encode_order_v3(&trade.taker_order));
    data
}

fn encode_order_v3(order: &OrderV3) -> Vec<Felt> {
    let mut data = Vec::new();
    data.push(order.account.0);
    data.push(order.market);
    data.push(order.side);
    data.push(order.orderType);
    data.push(order.size);
    data.push(order.price);
    data.push(order.signature_timestamp);
    data.push(if order.is_reduce_only { Felt::ONE } else { Felt::ZERO });
    data.extend(encode_order_category(&order.order_category));
    data
}

fn encode_order_category(cat: &OrderCategory) -> Vec<Felt> {
    match cat {
        OrderCategory::Unspecified => vec![Felt::from(0u64)],
        OrderCategory::API => vec![Felt::from(1u64)],
        OrderCategory::RPI => vec![Felt::from(2u64)],
        OrderCategory::Interactive => vec![Felt::from(3u64)],
        OrderCategory::Dynamic(fee) => vec![Felt::from(4u64), fee.fee, fee.fee_cap, fee.fee_floor],
        OrderCategory::DynamicWithToken(fee) => {
            vec![Felt::from(5u64), fee.fee, fee.fee_cap, fee.fee_floor, fee.fee_token.0]
        }
    }
}

fn validate_trade_basic(trade: &TradeRequestV3) -> Result<(), ExecutionError> {
    let size = felt_to_i128(trade.size)?;
    if size <= 1 {
        return Err(ExecutionError::ExecutionFailed("Trade: Size must be >1".to_string()));
    }
    let maker_size = felt_to_i128(trade.maker_order.size)?;
    let taker_size = felt_to_i128(trade.taker_order.size)?;
    if maker_size < size {
        return Err(ExecutionError::ExecutionFailed("Trade: invalid maker order size".to_string()));
    }
    if taker_size < size {
        return Err(ExecutionError::ExecutionFailed("Trade: invalid taker order size".to_string()));
    }
    if trade.traded_at == Felt::ZERO {
        return Err(ExecutionError::ExecutionFailed("Trade: traded_at must be >0".to_string()));
    }
    if trade.maker_order.market != trade.taker_order.market {
        return Err(ExecutionError::ExecutionFailed("Trade: Order assets mismatch".to_string()));
    }
    if trade.maker_order.account == trade.taker_order.account {
        return Err(ExecutionError::ExecutionFailed("Trade: self-trades not allowed".to_string()));
    }
    if trade.maker_order.side == trade.taker_order.side {
        return Err(ExecutionError::ExecutionFailed("Trade: same side orders".to_string()));
    }
    if trade.maker_order.side != buy_side() && trade.maker_order.side != sell_side() {
        return Err(ExecutionError::ExecutionFailed("Trade: invalid maker order side".to_string()));
    }
    if trade.taker_order.side != buy_side() && trade.taker_order.side != sell_side() {
        return Err(ExecutionError::ExecutionFailed("Trade: invalid taker order side".to_string()));
    }
    Ok(())
}

#[allow(non_snake_case, dead_code)]
mod ErrorCodes {
    use super::*;
    pub fn trade_order_risky() -> Felt {
        Felt::from(1001u64)
    }
    pub fn liquidation_account_healthy() -> Felt {
        Felt::from(1002u64)
    }
    pub fn liquidation_not_insurance_fund() -> Felt {
        Felt::from(1003u64)
    }
    pub fn trade_market_not_supported() -> Felt {
        Felt::from(1004u64)
    }
    pub fn liquidation_no_positions() -> Felt {
        Felt::from(1005u64)
    }
    pub fn reduce_only_will_increase_maker() -> Felt {
        Felt::from(1006u64)
    }
    pub fn reduce_only_will_increase_taker() -> Felt {
        Felt::from(1007u64)
    }
    pub fn trade_maker_not_enough_balance() -> Felt {
        Felt::from(1008u64)
    }
    pub fn trade_taker_not_enough_balance() -> Felt {
        Felt::from(1009u64)
    }
    pub fn trade_maker_not_enough_fee_token() -> Felt {
        Felt::from(1012u64)
    }
    pub fn trade_taker_not_enough_fee_token() -> Felt {
        Felt::from(1013u64)
    }
}

#[allow(non_snake_case, dead_code)]
mod Errors {
    use super::*;
    pub fn trade_size_too_small() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: Size must be >1")
    }
    pub fn trade_time_invalid_not_zero() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: traded_at must be >0")
    }
    pub fn trade_time_invalid_not_future() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: traded_at from future")
    }
    pub fn trade_market_mismatch() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: Order assets mismatch")
    }
    pub fn trade_market_not_supported() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: Market not supported")
    }
    pub fn trade_order_risky_taker() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: Too risky for taker")
    }
    pub fn trade_order_risky_maker() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: Too risky for maker")
    }
    pub fn trade_same_account() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: self-trades not allowed")
    }
    pub fn trade_same_side() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: same side orders")
    }
    pub fn reduce_only_will_increase_maker() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: reduce-only for maker")
    }
    pub fn reduce_only_will_increase_taker() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: reduce-only for taker")
    }
    pub fn trade_invalid_maker_size() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: invalid maker order size")
    }
    pub fn trade_invalid_taker_size() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: invalid taker order size")
    }
    pub fn trade_invalid_maker_side() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: invalid maker order side")
    }
    pub fn trade_invalid_taker_side() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: invalid taker order side")
    }
    pub fn trade_maker_too_many_assets() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: maker too many assets")
    }
    pub fn trade_taker_too_many_assets() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: taker too many assets")
    }
    pub fn trade_maker_not_enough_balance() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: maker not enough balance")
    }
    pub fn trade_taker_not_enough_balance() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: taker not enough balance")
    }
    pub fn trade_maker_not_enough_fee_token() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: maker insuff fee token")
    }
    pub fn trade_taker_not_enough_fee_token() -> Felt {
        crate::core::storage::short_string_to_felt("Trade: taker insuff fee token")
    }
}

fn buy_side() -> Felt {
    Felt::ONE
}

fn sell_side() -> Felt {
    Felt::from(2u64)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub const MULTIPLIER_FOR_TEST: i128 = MULTIPLIER;

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn buy_side_for_test() -> Felt {
    buy_side()
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn sell_side_for_test() -> Felt {
    sell_side()
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn felt_to_i128_for_test(value: Felt) -> Result<i128, ExecutionError> {
    felt_to_i128(value)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn i128_to_felt_for_test(value: i128) -> Felt {
    i128_to_felt(value)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn mul_128_scaled_for_test(a: i128, b: i128) -> Result<i128, ExecutionError> {
    mul_128_scaled(a, b)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn div_128_scaled_for_test(a: i128, b: i128) -> Result<i128, ExecutionError> {
    div_128_scaled(a, b)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn mul3_128_scaled_for_test(a: i128, b: i128, c: i128) -> Result<i128, ExecutionError> {
    mul3_128_scaled(a, b, c)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn abs_128_for_test(value: i128) -> i128 {
    abs_128(value)
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
pub fn validate_trade_basic_for_test(trade: &TradeRequestV3) -> Result<(), ExecutionError> {
    validate_trade_basic(trade)
}
