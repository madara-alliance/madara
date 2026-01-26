//! HeavyTradeSimulator function implementations.
//!
//! Implements the main trading simulation with 80-100 Pedersen hashes.

use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

use super::layout;
use super::types::{AccountState, AssetBalance, FeeConfig, MarketState, MULTIPLIER, MIN_MARGIN_RATIO};
use crate::context::ExecutionContext;
use crate::contracts::ExecutionError;
use crate::state::StateReader;
use crate::storage::{sn_keccak, short_string_to_felt};
use crate::types::ContractAddress;

/// Execute heavy_trade_simulation function.
///
/// This simulates the computational intensity of settle_trade_v3:
/// - 80-100 Pedersen hash operations (storage reads/writes)
/// - 10-15 Poseidon hash operations (events)
/// - Complex mathematical operations
/// - Multiple state changes
pub fn execute_heavy_trade_simulation<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    maker: ContractAddress,
    taker: ContractAddress,
    market_id: Felt,
    trade_size: u128,
    trade_price: u128,
    is_spot: bool,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let mut hash_count: u128 = 0;

    // DEBUG: Log input parameters and computed keys
    tracing::info!(
        "🔑 RUST EXEC INPUT: maker={:#x}, taker={:#x}, market_id={:#x}, trade_size={}, is_spot={}",
        maker.0, taker.0, market_id, trade_size, is_spot
    );

    // Print base addresses to compare with Blockifier
    tracing::info!(
        "🔑 BASE: account_asset_balances={:#x}",
        *layout::ACCOUNT_ASSET_BALANCES_BASE
    );

    // Compute trade_id = pedersen(maker, taker)
    let trade_id = Pedersen::hash(&maker.0, &taker.0);
    hash_count += 1;

    // ========================================================================
    // PHASE 1: Load Market State (3-5 Pedersen hashes)
    // ========================================================================
    let market_state_key = layout::market_state_key(market_id);
    let market_state_values = read_struct(state, contract_address, market_state_key, 5, ctx)?;
    let mut market_state = MarketState::from_storage(&market_state_values);
    hash_count += 1;

    let fee_config_key = layout::market_fee_config_key(market_id);
    let fee_config_values = read_struct(state, contract_address, fee_config_key, 4, ctx)?;
    let fee_config = FeeConfig::from_storage(&fee_config_values);
    hash_count += 1;

    let oracle_key = layout::market_oracle_key(market_id);
    let _oracle_address = ctx.storage_read(state, contract_address, oracle_key)?;
    hash_count += 1;

    // Emit OracleConsulted event (no timestamp in idempotent version)
    emit_oracle_consulted(ctx, market_id, trade_price);

    // ========================================================================
    // PHASE 2: Load Account States (10-15 Pedersen hashes per account)
    // ========================================================================
    let (mut maker_state, maker_hash_count) = load_account_state(state, contract_address, maker, ctx)?;
    hash_count += maker_hash_count;

    let (mut taker_state, taker_hash_count) = load_account_state(state, contract_address, taker, ctx)?;
    hash_count += taker_hash_count;

    // ========================================================================
    // PHASE 3: Load Asset Balances (4-8 Pedersen hashes)
    // ========================================================================
    let maker_balance_key = layout::account_asset_balance_key(maker, market_id);
    let maker_balance_values = read_struct(state, contract_address, maker_balance_key, 2, ctx)?;
    let maker_balance = AssetBalance::from_storage(&maker_balance_values);
    hash_count += 2;

    // DEBUG: Log storage keys and values
    tracing::info!(
        "🔑 STORAGE DEBUG: maker_balance_key={:#x}, amount_read={}, locked_read={}",
        maker_balance_key.0, maker_balance.amount, maker_balance.locked_amount
    );

    let taker_balance_key = layout::account_asset_balance_key(taker, market_id);
    let taker_balance_values = read_struct(state, contract_address, taker_balance_key, 2, ctx)?;
    let taker_balance = AssetBalance::from_storage(&taker_balance_values);
    hash_count += 2;

    tracing::info!(
        "🔑 STORAGE DEBUG: taker_balance_key={:#x}, amount_read={}, locked_read={}",
        taker_balance_key.0, taker_balance.amount, taker_balance.locked_amount
    );

    // Also log the total_hashes_computed key for comparison
    tracing::info!(
        "🔑 STORAGE DEBUG: total_hashes_computed_key={:#x}",
        layout::TOTAL_HASHES_COMPUTED_KEY.0
    );

    // ========================================================================
    // PHASE 4: Load Referrers (2 Pedersen hashes)
    // ========================================================================
    let maker_referrer_key = layout::account_referrer_key(maker);
    let maker_referrer = ctx.storage_read(state, contract_address, maker_referrer_key)?;
    hash_count += 1;

    let taker_referrer_key = layout::account_referrer_key(taker);
    let taker_referrer = ctx.storage_read(state, contract_address, taker_referrer_key)?;
    hash_count += 1;

    // ========================================================================
    // PHASE 5: Complex Fee Calculations
    // ========================================================================
    let notional_value = (trade_size * trade_price) / MULTIPLIER;
    let maker_fee = (notional_value * fee_config.maker_fee_rate) / 10000;
    let taker_fee = (notional_value * fee_config.taker_fee_rate) / 10000;
    let total_fees = maker_fee + taker_fee;
    let referral_amount = (total_fees * fee_config.referral_rate) / 100;
    let insurance_amount = (total_fees * fee_config.insurance_fund_rate) / 100;

    // ========================================================================
    // PHASE 6: Compute Multiple Pedersen Hashes (simulate state hashing)
    // INCREASED from 20 to 300 hashes for heavier computation
    // ========================================================================
    let mut hash_accumulator = trade_id;
    for i in 0..300u128 {
        hash_accumulator = Pedersen::hash(&hash_accumulator, &Felt::from(i));
        hash_count += 1;

        // Emit hash computation event every 50 iterations
        if (i + 1) % 50 == 0 {
            emit_hash_computation_event(ctx, i + 1, short_string_to_felt("hash_batch"));
        }
    }

    // ========================================================================
    // PHASE 7: Position and PnL Calculations (for perpetual trades)
    // ========================================================================
    if !is_spot {
        let maker_position_key = layout::perpetual_position_key(maker, market_id);
        let maker_position_felt = ctx.storage_read(state, contract_address, maker_position_key)?;
        let mut maker_position = felt_to_i128(maker_position_felt);
        hash_count += 2;

        let taker_position_key = layout::perpetual_position_key(taker, market_id);
        let taker_position_felt = ctx.storage_read(state, contract_address, taker_position_key)?;
        let mut taker_position = felt_to_i128(taker_position_felt);
        hash_count += 2;

        // Calculate PnL
        let maker_pnl = calculate_pnl(maker_position, trade_price, trade_size);
        let taker_pnl = calculate_pnl(taker_position, trade_price, trade_size);

        // Update positions
        let trade_size_i128 = trade_size as i128;
        let old_maker_position = maker_position;
        let old_taker_position = taker_position;
        maker_position -= trade_size_i128;
        taker_position += trade_size_i128;

        // Write new positions
        ctx.storage_write(contract_address, maker_position_key, i128_to_felt(maker_position));
        hash_count += 2;

        ctx.storage_write(contract_address, taker_position_key, i128_to_felt(taker_position));
        hash_count += 2;

        // Update realized PnL
        let maker_pnl_key = layout::perpetual_realized_pnl_key(maker, market_id);
        ctx.storage_write(contract_address, maker_pnl_key, i128_to_felt(maker_pnl));
        hash_count += 2;

        let taker_pnl_key = layout::perpetual_realized_pnl_key(taker, market_id);
        ctx.storage_write(contract_address, taker_pnl_key, i128_to_felt(taker_pnl));
        hash_count += 2;

        // Update funding paid
        let funding = calculate_funding(maker_position, market_state.funding_rate);
        let maker_funding_key = layout::perpetual_funding_paid_key(maker, market_id);
        ctx.storage_write(contract_address, maker_funding_key, i128_to_felt(funding));
        hash_count += 2;

        let taker_funding_key = layout::perpetual_funding_paid_key(taker, market_id);
        ctx.storage_write(contract_address, taker_funding_key, i128_to_felt(-funding));
        hash_count += 2;

        // Emit events
        emit_perpetual_position_updated(ctx, maker, market_id, old_maker_position, maker_position, maker_pnl);
        emit_perpetual_position_updated(ctx, taker, market_id, old_taker_position, taker_position, taker_pnl);
        emit_funding_paid(ctx, maker, funding, market_id);
    }

    // ========================================================================
    // PHASE 8: Risk Checks (margin ratio calculations)
    // ========================================================================
    let maker_margin_ratio = calculate_margin_ratio(&maker_state, notional_value);
    let taker_margin_ratio = calculate_margin_ratio(&taker_state, notional_value);

    if maker_margin_ratio < MIN_MARGIN_RATIO {
        emit_margin_call(ctx, maker, maker_margin_ratio, MIN_MARGIN_RATIO);
    }

    if taker_margin_ratio < MIN_MARGIN_RATIO {
        emit_margin_call(ctx, taker, taker_margin_ratio, MIN_MARGIN_RATIO);
    }

    // ========================================================================
    // PHASE 9: Update Account States (8-12 Pedersen hashes)
    // ========================================================================
    maker_state.balance = maker_state.balance.saturating_sub(maker_fee);
    maker_state.trade_count += 1;
    maker_state.total_fees_paid += maker_fee;

    taker_state.balance = taker_state.balance.saturating_sub(taker_fee);
    taker_state.trade_count += 1;
    taker_state.total_fees_paid += taker_fee;

    write_account_state(ctx, contract_address, maker, &maker_state);
    hash_count += 1;

    write_account_state(ctx, contract_address, taker, &taker_state);
    hash_count += 1;

    // ========================================================================
    // PHASE 10: Update Asset Balances (4 hashes)
    // ========================================================================
    let new_maker_amount = maker_balance.amount.saturating_sub(trade_size);
    let new_taker_amount = taker_balance.amount + trade_size;

    // DEBUG: Log what we're computing
    tracing::debug!(
        "🔍 DEBUG: Asset balance update - trade_size={}, maker: {} -> {}, taker: {} -> {}",
        trade_size,
        maker_balance.amount,
        new_maker_amount,
        taker_balance.amount,
        new_taker_amount
    );

    let maker_write_key = layout::account_asset_balance_key(maker, market_id);
    tracing::info!(
        "🔑 WRITE: maker_asset_balance key={:#x}, old_amount={}, new_amount={}",
        maker_write_key.0, maker_balance.amount, new_maker_amount
    );
    write_asset_balance(ctx, contract_address, maker, market_id, new_maker_amount, maker_balance.locked_amount);
    hash_count += 2;

    let taker_write_key = layout::account_asset_balance_key(taker, market_id);
    tracing::info!(
        "🔑 WRITE: taker_asset_balance key={:#x}, old_amount={}, new_amount={}",
        taker_write_key.0, taker_balance.amount, new_taker_amount
    );
    write_asset_balance(ctx, contract_address, taker, market_id, new_taker_amount, taker_balance.locked_amount);
    hash_count += 2;

    // ========================================================================
    // PHASE 11: Fee Distribution Events
    // ========================================================================
    if maker_referrer != Felt::ZERO {
        emit_referral_paid(ctx, ContractAddress(maker_referrer), maker, referral_amount / 2);
    }

    if taker_referrer != Felt::ZERO {
        emit_referral_paid(ctx, ContractAddress(taker_referrer), taker, referral_amount / 2);
    }

    emit_insurance_fund_contribution(ctx, insurance_amount, insurance_amount);

    // ========================================================================
    // PHASE 12: Update Market State (1 hash)
    // ========================================================================
    market_state.total_volume += notional_value;
    market_state.last_price = trade_price;
    if !is_spot {
        market_state.open_interest += trade_size as i128;
    }

    write_market_state(ctx, contract_address, market_id, &market_state);
    hash_count += 1;

    // ========================================================================
    // PHASE 13: Update Global Counters
    // ========================================================================
    let total_trades_key = *layout::TOTAL_TRADES_SETTLED_KEY;
    let current_trades = ctx.storage_read(state, contract_address, total_trades_key)?;
    ctx.storage_write(contract_address, total_trades_key, current_trades + Felt::ONE);

    let total_vol_key = *layout::TOTAL_VOLUME_KEY;
    let current_vol = ctx.storage_read(state, contract_address, total_vol_key)?;
    ctx.storage_write(contract_address, total_vol_key, current_vol + Felt::from(notional_value));

    // ========================================================================
    // PHASE 14: Store hash count (first write - matches Cairo contract)
    // ========================================================================
    let hash_key = *layout::TOTAL_HASHES_COMPUTED_KEY;
    let current_hashes = ctx.storage_read(state, contract_address, hash_key)?;
    let current_hashes_u128 = felt_to_u128(current_hashes);
    let hash_count_at_phase_14 = hash_count;
    ctx.storage_write(contract_address, hash_key, Felt::from(current_hashes_u128 + hash_count_at_phase_14));

    // ========================================================================
    // PHASE 15: Emit All Events
    // ========================================================================
    emit_trade_settled(ctx, trade_id, maker, taker, market_id, trade_size, trade_price, maker_fee, taker_fee, is_spot);
    emit_balance_updated(ctx, maker, market_id, maker_balance.amount, new_maker_amount);
    emit_balance_updated(ctx, taker, market_id, taker_balance.amount, new_taker_amount);
    emit_fee_collected(ctx, maker, short_string_to_felt("maker_fee"), maker_fee);
    emit_fee_collected(ctx, taker, short_string_to_felt("taker_fee"), taker_fee);
    emit_market_state_updated(ctx, market_id, market_state.total_volume, market_state.open_interest, market_state.last_price);
    emit_account_state_updated(ctx, maker, maker_state.balance, maker_state.realized_pnl, maker_state.trade_count);
    emit_account_state_updated(ctx, taker, taker_state.balance, taker_state.realized_pnl, taker_state.trade_count);
    emit_hash_computation_event(ctx, hash_count, short_string_to_felt("trade_settlement"));

    // ========================================================================
    // PHASE 16: Additional Computation - Risk Analysis Hashing (200 more)
    // ========================================================================
    let mut risk_hash = hash_accumulator;
    for j in 0..200u128 {
        risk_hash = Pedersen::hash(&risk_hash, &Felt::from(j + 1000));
        hash_count += 1;

        // Emit risk analysis events every 40 iterations
        if (j + 1) % 40 == 0 {
            emit_hash_computation_event(ctx, j + 1, short_string_to_felt("risk_analysis"));
        }
    }

    // ========================================================================
    // PHASE 17: Additional Storage Operations - Historical Data (40 more hashes)
    // ========================================================================
    for k in 0..20u128 {
        // Simulate writing trade history (2 hashes per write)
        let _history_key = Pedersen::hash(&trade_id, &Felt::from(k));
        let _history_value = Pedersen::hash(&risk_hash, &Felt::from(trade_size + k));
        hash_count += 2;

        // Emit data logging events every 5 iterations
        if (k + 1) % 5 == 0 {
            emit_hash_computation_event(ctx, k + 1, short_string_to_felt("history_log"));
        }
    }

    // ========================================================================
    // PHASE 18: Fee Distribution Verification Hashing (150 more hashes)
    // ========================================================================
    let mut fee_hash = Pedersen::hash(&Felt::from(maker_fee), &Felt::from(taker_fee));
    hash_count += 1;
    for m in 0..150u128 {
        fee_hash = Pedersen::hash(&fee_hash, &Felt::from(m + 2000));
        hash_count += 1;

        // Emit fee verification events every 30 iterations
        if (m + 1) % 30 == 0 {
            emit_hash_computation_event(ctx, m + 1, short_string_to_felt("fee_verify"));
        }
    }

    // Emit final computation summary
    emit_hash_computation_event(ctx, hash_count, short_string_to_felt("final_total"));

    // Store total hash count (second write - matches Cairo's self.total_hashes_computed.read() + hash_count)
    // Cairo reads the value from PHASE 14's write and adds current hash_count
    // Since we can't read our own pending writes, compute: (initial + phase14_count) + final_count
    let phase_14_written_value = current_hashes_u128 + hash_count_at_phase_14;
    let final_hash_value = phase_14_written_value + hash_count;
    ctx.storage_write(contract_address, hash_key, Felt::from(final_hash_value));

    // Return true (success)
    ctx.set_retdata(vec![Felt::ONE]);
    Ok(())
}

/// Execute get_account_state view function.
pub fn execute_get_account_state<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    account: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let key = layout::account_state_key(account);
    let values = read_struct(state, contract_address, key, 7, ctx)?;
    ctx.set_retdata(values);
    Ok(())
}

/// Execute get_market_state view function.
pub fn execute_get_market_state<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    market_id: Felt,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let key = layout::market_state_key(market_id);
    let values = read_struct(state, contract_address, key, 5, ctx)?;
    ctx.set_retdata(values);
    Ok(())
}

/// Execute get_total_hashes_computed view function.
pub fn execute_get_total_hashes_computed<S: StateReader>(
    state: &S,
    contract_address: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    let key = *layout::TOTAL_HASHES_COMPUTED_KEY;
    let value = ctx.storage_read(state, contract_address, key)?;
    ctx.set_retdata(vec![value]);
    Ok(())
}

/// Execute initialize_account function.
///
/// Initializes account state and asset balance for a market.
pub fn execute_initialize_account(
    contract_address: ContractAddress,
    account: ContractAddress,
    market_id: Felt,
    initial_balance: u128,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // Initialize account state (7 fields, no timestamp)
    let account_state = AccountState {
        balance: initial_balance,
        perpetual_position: 0,
        realized_pnl: 0,
        total_fees_paid: 0,
        trade_count: 0,
        margin_ratio: MULTIPLIER, // 100%
        is_active: true,
    };
    write_account_state(ctx, contract_address, account, &account_state);

    // Initialize asset balance for the market (2 fields, no timestamp)
    write_asset_balance(ctx, contract_address, account, market_id, initial_balance, 0);

    Ok(())
}

/// Execute initialize_market function.
///
/// Initializes market state and fee configuration.
pub fn execute_initialize_market(
    contract_address: ContractAddress,
    market_id: Felt,
    ctx: &mut ExecutionContext,
) -> Result<(), ExecutionError> {
    // Initialize market state
    let market_state = MarketState {
        total_volume: 0,
        open_interest: 0,
        last_price: 50000, // Default price as in Cairo
        funding_rate: 0,
        is_enabled: true,
    };
    write_market_state(ctx, contract_address, market_id, &market_state);

    // Set default fee config for the market
    let fee_config = FeeConfig {
        maker_fee_rate: 5,  // 0.05%
        taker_fee_rate: 10, // 0.10%
        referral_rate: 20,  // 20% of fees
        insurance_fund_rate: 10, // 10% of fees
    };
    write_fee_config(ctx, contract_address, market_id, &fee_config);

    Ok(())
}

fn write_fee_config(
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    market_id: Felt,
    config: &FeeConfig,
) {
    let key = layout::market_fee_config_key(market_id);
    ctx.storage_write(contract, key, Felt::from(config.maker_fee_rate));
    ctx.storage_write(contract, crate::types::StorageKey(key.0 + Felt::ONE), Felt::from(config.taker_fee_rate));
    ctx.storage_write(contract, crate::types::StorageKey(key.0 + Felt::TWO), Felt::from(config.referral_rate));
    ctx.storage_write(contract, crate::types::StorageKey(key.0 + Felt::THREE), Felt::from(config.insurance_fund_rate));
}

// ============================================================================
// Helper Functions
// ============================================================================

fn read_struct<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    base_key: crate::types::StorageKey,
    num_fields: usize,
    ctx: &mut ExecutionContext,
) -> Result<Vec<Felt>, ExecutionError> {
    let mut values = Vec::with_capacity(num_fields);
    for i in 0..num_fields {
        let key = crate::types::StorageKey(base_key.0 + Felt::from(i as u64));
        values.push(ctx.storage_read(state, contract, key)?);
    }
    Ok(values)
}

fn load_account_state<S: StateReader>(
    state: &S,
    contract: ContractAddress,
    account: ContractAddress,
    ctx: &mut ExecutionContext,
) -> Result<(AccountState, u128), ExecutionError> {
    let mut hash_count = 0u128;

    // Read account state struct (7 fields, no timestamp)
    let key = layout::account_state_key(account);
    let values = read_struct(state, contract, key, 7, ctx)?;
    let account_state = AccountState::from_storage(&values);
    hash_count += 1;

    // Read fee override
    let fee_override_key = layout::account_fee_override_key(account);
    let _ = ctx.storage_read(state, contract, fee_override_key)?;
    hash_count += 1;

    // Simulate loading multiple positions (4 extra hashes)
    let mut temp_hash = account.0;
    for i in 0..4u128 {
        temp_hash = Pedersen::hash(&temp_hash, &Felt::from(i));
        hash_count += 1;
    }

    Ok((account_state, hash_count))
}

fn write_account_state(
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    account: ContractAddress,
    state: &AccountState,
) {
    let key = layout::account_state_key(account);
    let values = state.to_storage();
    for (i, value) in values.iter().enumerate() {
        let slot_key = crate::types::StorageKey(key.0 + Felt::from(i as u64));
        ctx.storage_write(contract, slot_key, *value);
    }
}

fn write_asset_balance(
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    account: ContractAddress,
    asset: Felt,
    amount: u128,
    locked: u128,
) {
    let key = layout::account_asset_balance_key(account, asset);
    ctx.storage_write(contract, key, Felt::from(amount));
    ctx.storage_write(contract, crate::types::StorageKey(key.0 + Felt::ONE), Felt::from(locked));
}

fn write_market_state(
    ctx: &mut ExecutionContext,
    contract: ContractAddress,
    market_id: Felt,
    state: &MarketState,
) {
    let key = layout::market_state_key(market_id);
    let values = state.to_storage();
    for (i, value) in values.iter().enumerate() {
        let slot_key = crate::types::StorageKey(key.0 + Felt::from(i as u64));
        ctx.storage_write(contract, slot_key, *value);
    }
}

fn calculate_pnl(position: i128, current_price: u128, _size: u128) -> i128 {
    let price_i128 = current_price as i128;
    (position * price_i128) / (MULTIPLIER as i128)
}

fn calculate_funding(position: i128, funding_rate: i128) -> i128 {
    (position * funding_rate) / (MULTIPLIER as i128)
}

fn calculate_margin_ratio(account_state: &AccountState, notional_value: u128) -> u128 {
    if notional_value == 0 {
        return MULTIPLIER; // 100%
    }
    (account_state.balance * MULTIPLIER) / notional_value
}

fn felt_to_u128(felt: Felt) -> u128 {
    let bytes = felt.to_bytes_be();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    u128::from_be_bytes(arr)
}

fn felt_to_i128(felt: Felt) -> i128 {
    let bytes = felt.to_bytes_be();
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes[16..32]);
    i128::from_be_bytes(arr)
}

fn i128_to_felt(value: i128) -> Felt {
    if value >= 0 {
        Felt::from(value as u128)
    } else {
        Felt::ZERO - Felt::from((-value) as u128)
    }
}

// ============================================================================
// Event Emission Helpers
// ============================================================================

fn emit_trade_settled(
    ctx: &mut ExecutionContext,
    trade_id: Felt,
    maker: ContractAddress,
    taker: ContractAddress,
    market_id: Felt,
    size: u128,
    price: u128,
    maker_fee: u128,
    taker_fee: u128,
    is_spot: bool,
) {
    let selector = sn_keccak(b"TradeSettled");
    ctx.emit_event(
        vec![selector, trade_id, maker.0, taker.0],
        vec![
            market_id,
            Felt::from(size),
            Felt::from(price),
            Felt::from(maker_fee),
            Felt::from(taker_fee),
            if is_spot { Felt::ONE } else { Felt::ZERO },
        ],
    );
}

fn emit_balance_updated(
    ctx: &mut ExecutionContext,
    account: ContractAddress,
    asset: Felt,
    old_balance: u128,
    new_balance: u128,
) {
    let selector = sn_keccak(b"BalanceUpdated");
    ctx.emit_event(
        vec![selector, account.0, asset],
        vec![Felt::from(old_balance), Felt::from(new_balance)],
    );
}

fn emit_perpetual_position_updated(
    ctx: &mut ExecutionContext,
    account: ContractAddress,
    market: Felt,
    old_position: i128,
    new_position: i128,
    realized_pnl: i128,
) {
    let selector = sn_keccak(b"PerpetualPositionUpdated");
    ctx.emit_event(
        vec![selector, account.0, market],
        vec![i128_to_felt(old_position), i128_to_felt(new_position), i128_to_felt(realized_pnl)],
    );
}

fn emit_fee_collected(ctx: &mut ExecutionContext, account: ContractAddress, fee_type: Felt, amount: u128) {
    let selector = sn_keccak(b"FeeCollected");
    ctx.emit_event(vec![selector, account.0], vec![fee_type, Felt::from(amount)]);
}

fn emit_referral_paid(ctx: &mut ExecutionContext, referrer: ContractAddress, referred: ContractAddress, amount: u128) {
    let selector = sn_keccak(b"ReferralPaid");
    ctx.emit_event(vec![selector, referrer.0, referred.0], vec![Felt::from(amount)]);
}

fn emit_insurance_fund_contribution(ctx: &mut ExecutionContext, amount: u128, total_fund: u128) {
    let selector = sn_keccak(b"InsuranceFundContribution");
    ctx.emit_event(vec![selector], vec![Felt::from(amount), Felt::from(total_fund)]);
}

fn emit_market_state_updated(ctx: &mut ExecutionContext, market_id: Felt, total_volume: u128, open_interest: i128, last_price: u128) {
    let selector = sn_keccak(b"MarketStateUpdated");
    ctx.emit_event(
        vec![selector, market_id],
        vec![Felt::from(total_volume), i128_to_felt(open_interest), Felt::from(last_price)],
    );
}

fn emit_account_state_updated(ctx: &mut ExecutionContext, account: ContractAddress, balance: u128, realized_pnl: i128, trade_count: u128) {
    let selector = sn_keccak(b"AccountStateUpdated");
    ctx.emit_event(
        vec![selector, account.0],
        vec![Felt::from(balance), i128_to_felt(realized_pnl), Felt::from(trade_count)],
    );
}

fn emit_hash_computation_event(ctx: &mut ExecutionContext, hash_count: u128, operation: Felt) {
    let selector = sn_keccak(b"HashComputationEvent");
    ctx.emit_event(vec![selector], vec![Felt::from(hash_count), operation]);
}

fn emit_oracle_consulted(ctx: &mut ExecutionContext, market: Felt, price: u128) {
    let selector = sn_keccak(b"OracleConsulted");
    ctx.emit_event(vec![selector, market], vec![Felt::from(price)]);
}

fn emit_funding_paid(ctx: &mut ExecutionContext, account: ContractAddress, amount: i128, market: Felt) {
    let selector = sn_keccak(b"FundingPaid");
    ctx.emit_event(vec![selector, account.0], vec![i128_to_felt(amount), market]);
}

fn emit_margin_call(ctx: &mut ExecutionContext, account: ContractAddress, current_ratio: u128, required_ratio: u128) {
    let selector = sn_keccak(b"MarginCall");
    ctx.emit_event(vec![selector, account.0], vec![Felt::from(current_ratio), Felt::from(required_ratio)]);
}
