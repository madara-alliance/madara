use super::super::fixtures::{
    addr, felt, i128_to_felt, set_oracle_funding_index, set_oracle_latest_tick_data, set_paraclear_dependencies,
    set_settlement_token, set_storage, set_token_name, short_str,
};
use crate::contracts::paradex::paraclear;
use crate::storage::{storage_key_for_map, storage_key_with_offset};

const SCALE: i128 = 100_000_000;

#[test]
fn test_get_asset_value_spot_only() {
    let amounts = vec![SCALE, 2 * SCALE];
    let prices = vec![2 * SCALE, 3 * SCALE];
    let value = paraclear::get_asset_value_for_test(&amounts, &prices).expect("asset value");
    assert_eq!(value, 8 * SCALE);
}

#[test]
fn test_get_asset_value_perp_only() {
    let amounts: Vec<i128> = Vec::new();
    let prices: Vec<i128> = Vec::new();
    let value = paraclear::get_asset_value_for_test(&amounts, &prices).expect("asset value");
    assert_eq!(value, 0);
}

#[test]
fn test_margin_requirement_and_total_upnl_basic() {
    let token_amounts: Vec<i128> = Vec::new();
    let token_prices: Vec<i128> = Vec::new();
    let settlement_token_index = 0usize;

    let perp_amounts = vec![2 * SCALE];
    let perp_costs = vec![SCALE];
    let perp_cached_funding = vec![3 * SCALE];
    let perp_prices = vec![5 * SCALE];
    let perp_funding_indices = vec![SCALE];
    let perp_imf_base = vec![SCALE];
    let perp_fee_provision_rate = vec![None];
    let fee_rate_taker = SCALE;
    let referrer = addr(0);
    let fee_commission = 0i128;
    let fee_discount = 0i128;
    let margin_methodology = 0u8; // CrossMargin
    let settlement_token_price = SCALE;
    let perp_mmf_factor = 0i128;

    let (margin_req, total_upnl) = paraclear::margin_requirement_and_total_upnl_for_test(
        &token_amounts,
        &token_prices,
        settlement_token_index,
        &perp_amounts,
        &perp_costs,
        &perp_cached_funding,
        &perp_prices,
        &perp_funding_indices,
        &perp_imf_base,
        &perp_fee_provision_rate,
        fee_rate_taker,
        referrer,
        fee_commission,
        fee_discount,
        margin_methodology,
        settlement_token_price,
        perp_mmf_factor,
        false,
    )
    .expect("margin");

    assert_eq!(margin_req, 2_000_000_000i128);
    assert_eq!(total_upnl, 1_300_000_000i128);
}

#[test]
fn test_unrealized_pnl_long_positive() {
    let pnl = paraclear::unrealized_pnl_for_test(2 * SCALE, SCALE, 3 * SCALE, SCALE).expect("pnl");
    assert_eq!(pnl, 5 * SCALE);
}

#[test]
fn test_unrealized_pnl_short_positive() {
    let pnl = paraclear::unrealized_pnl_for_test(-2 * SCALE, -3 * SCALE, SCALE, SCALE).expect("pnl");
    assert_eq!(pnl, SCALE);
}

#[test]
fn test_unrealized_funding_pnl_positive() {
    // amount=2, cached_funding=3, funding_index=1, settlement_token_price=1
    // funding_diff = 3 - 1 = 2, pnl_usdc = 2 * 2 = 4, pnl_usd = 4 * 1 = 4
    let pnl = paraclear::unrealized_funding_pnl_for_test(2 * SCALE, 3 * SCALE, SCALE, SCALE).expect("funding pnl");
    assert_eq!(pnl, 4 * SCALE);
}

#[test]
fn test_unrealized_funding_pnl_with_settlement_price() {
    // amount=2, cached_funding=3, funding_index=1, settlement_token_price=2
    // funding_diff = 3 - 1 = 2
    // pnl_usdc = funding_diff * amount = 2 * 2 = 4
    // pnl_usd = pnl_usdc * settlement_token_price = 4 * 2 = 8
    let pnl = paraclear::unrealized_funding_pnl_for_test(2 * SCALE, 3 * SCALE, SCALE, 2 * SCALE).expect("funding pnl");
    assert_eq!(pnl, 8 * SCALE);
}

#[test]
fn test_unrealized_funding_pnl_negative() {
    // amount=2, cached_funding=1, funding_index=3, settlement_token_price=1
    // funding_diff = 1 - 3 = -2, pnl_usdc = -2 * 2 = -4, pnl_usd = -4 * 1 = -4
    let pnl = paraclear::unrealized_funding_pnl_for_test(2 * SCALE, SCALE, 3 * SCALE, SCALE).expect("funding pnl");
    assert_eq!(pnl, -4 * SCALE);
}

#[test]
fn test_margin_requirement_includes_funding_pnl_with_settlement_price() {
    // With settlement_token_price=2, the funding PnL should be doubled
    // compared to settlement_token_price=1
    let token_amounts: Vec<i128> = Vec::new();
    let token_prices: Vec<i128> = Vec::new();
    let settlement_token_index = 0usize;

    let perp_amounts = vec![2 * SCALE];
    let perp_costs = vec![SCALE];
    let perp_cached_funding = vec![3 * SCALE];
    let perp_prices = vec![5 * SCALE];
    let perp_funding_indices = vec![SCALE];
    let perp_imf_base = vec![SCALE];
    let perp_fee_provision_rate = vec![None];
    let fee_rate_taker = SCALE;
    let referrer = addr(0);
    let fee_commission = 0i128;
    let fee_discount = 0i128;
    let margin_methodology = 0u8; // CrossMargin
    let perp_mmf_factor = 0i128;

    // settlement_token_price=1: unrealized_pnl=9, funding_pnl=4, total_upnl=13
    let (_margin_1, total_upnl_1) = paraclear::margin_requirement_and_total_upnl_for_test(
        &token_amounts,
        &token_prices,
        settlement_token_index,
        &perp_amounts,
        &perp_costs,
        &perp_cached_funding,
        &perp_prices,
        &perp_funding_indices,
        &perp_imf_base,
        &perp_fee_provision_rate,
        fee_rate_taker,
        referrer,
        fee_commission,
        fee_discount,
        margin_methodology,
        SCALE,
        perp_mmf_factor,
        false,
    )
    .expect("margin");

    // settlement_token_price=2: unrealized_pnl=8 (cost*2=2), funding_pnl=8 (4*2=8), total_upnl=16
    let (_margin_2, total_upnl_2) = paraclear::margin_requirement_and_total_upnl_for_test(
        &token_amounts,
        &token_prices,
        settlement_token_index,
        &perp_amounts,
        &perp_costs,
        &perp_cached_funding,
        &perp_prices,
        &perp_funding_indices,
        &perp_imf_base,
        &perp_fee_provision_rate,
        fee_rate_taker,
        referrer,
        fee_commission,
        fee_discount,
        margin_methodology,
        2 * SCALE,
        perp_mmf_factor,
        false,
    )
    .expect("margin");

    // funding_pnl at price=1 is 4*SCALE, at price=2 is 8*SCALE
    // so total_upnl_2 should differ from total_upnl_1 by 4*SCALE (the extra funding)
    // plus unrealized_pnl also changes (cost * settlement_price changes)
    assert!(total_upnl_2 > total_upnl_1, "higher settlement price should increase total_upnl");
    assert_eq!(total_upnl_1, 1_300_000_000i128); // same as existing test
}

#[test]
fn test_is_risky_after_trade_spot_false() {
    let mut state = crate::state::mock::MockStateReader::new();
    let contract = addr(0x6000);
    let assets_manager = addr(0x6001);
    let oracle = addr(0x6002);
    let account = addr(0x6003);
    let base_token = addr(0x6004);
    let settlement_token = addr(0x6005);
    let base_name = short_str("AAA");
    let settlement_name = short_str("USDC");

    set_paraclear_dependencies(&mut state, contract, assets_manager, oracle);
    set_settlement_token(&mut state, contract, settlement_token, settlement_name);
    set_token_name(&mut state, contract, base_token, base_name);
    set_oracle_latest_tick_data(&mut state, oracle, base_name, base_name, i128_to_felt(SCALE), felt(8));
    set_oracle_latest_tick_data(&mut state, oracle, settlement_name, settlement_name, i128_to_felt(SCALE), felt(8));

    let mut ctx = crate::ExecutionContext::new();
    let risky = paraclear::is_risky_after_trade_spot_for_test(
        &mut ctx,
        &state,
        contract,
        account,
        settlement_token,
        base_token,
        0,
        SCALE,
        2 * SCALE,
        0,
    )
    .expect("risk");

    assert!(!risky);
}

#[test]
fn test_is_risky_after_trade_perp_true() {
    let mut state = crate::state::mock::MockStateReader::new();
    let contract = addr(0x6100);
    let assets_manager = addr(0x6101);
    let oracle = addr(0x6102);
    let account = addr(0x6103);
    let market = felt(0x6104);
    let settlement_token = addr(0x6105);
    let settlement_name = short_str("USDC");

    set_paraclear_dependencies(&mut state, contract, assets_manager, oracle);
    set_settlement_token(&mut state, contract, settlement_token, settlement_name);
    set_oracle_latest_tick_data(&mut state, oracle, market, market, i128_to_felt(SCALE), felt(8));
    set_oracle_latest_tick_data(&mut state, oracle, settlement_name, settlement_name, i128_to_felt(SCALE), felt(8));
    set_oracle_funding_index(&mut state, oracle, market, i128_to_felt(0));

    let perp_base = storage_key_for_map("Paraclear_perpetual_asset", market);
    set_storage(&mut state, contract, perp_base, market);
    set_storage(&mut state, contract, storage_key_with_offset(perp_base, 4), i128_to_felt(SCALE));

    let mut ctx = crate::ExecutionContext::new();
    let risky = paraclear::is_risky_after_trade_perp_for_test(
        &mut ctx,
        &state,
        contract,
        account,
        settlement_token,
        market,
        0,
        5 * SCALE,
        felt(0),
        felt(0),
        0,
        SCALE,
    )
    .expect("risk");

    assert!(risky);
}
