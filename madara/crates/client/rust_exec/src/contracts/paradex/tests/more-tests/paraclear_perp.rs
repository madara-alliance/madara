#![allow(clippy::identity_op, clippy::too_many_arguments)]

use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_types::{FeeWithCapRequest, OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::{
    event_selector, function_selector, storage_key_for_map, storage_key_for_map2, storage_key_with_offset,
};
use crate::types::ContractAddress;

use super::super::fixtures::{
    addr, felt, i128_to_felt, set_account_fee_rate_future, set_assets_manager_market_fee_config,
    set_future_asset_direct, set_option_asset_direct, set_oracle_funding_index, set_oracle_latest_tick_data,
    set_paraclear_dependencies, set_settlement_token, set_token_balance_amount_only, set_token_name, short_str, SCALE,
};

fn setup_perp_env(
    state: &mut MockStateReader,
    contract: ContractAddress,
    assets_manager: ContractAddress,
    oracle: ContractAddress,
    market: starknet_types_core::felt::Felt,
    settlement_token: ContractAddress,
    settlement_name: starknet_types_core::felt::Felt,
    mark_price: i128,
    settlement_price: i128,
    funding_index: i128,
    imf_base: i128,
    is_option: bool,
) {
    set_paraclear_dependencies(state, contract, assets_manager, oracle);
    set_settlement_token(state, contract, settlement_token, settlement_name);
    set_token_name(state, contract, settlement_token, settlement_name);

    if is_option {
        set_option_asset_direct(state, assets_manager, market, felt(0x1), felt(0x2), felt(1), felt(0), felt(0));
    } else {
        set_future_asset_direct(
            state,
            assets_manager,
            market,
            felt(0x1),
            felt(0x2),
            felt(1),
            felt(0),
            felt(0),
            felt(0),
            felt(0),
        );
    }

    let perp_base = storage_key_for_map("Paraclear_perpetual_asset", market);
    state.set_storage(contract, perp_base, market);
    state.set_storage(contract, storage_key_with_offset(perp_base, 4), i128_to_felt(imf_base));

    set_oracle_latest_tick_data(state, oracle, market, market, i128_to_felt(mark_price), felt(8));
    set_oracle_latest_tick_data(
        state,
        oracle,
        settlement_name,
        settlement_name,
        i128_to_felt(settlement_price),
        felt(8),
    );
    set_oracle_funding_index(state, oracle, market, i128_to_felt(funding_index));
}

fn set_perp_balance(
    state: &mut MockStateReader,
    contract: ContractAddress,
    account: ContractAddress,
    market: starknet_types_core::felt::Felt,
    amount: i128,
    cost: i128,
    cached_funding: i128,
) {
    let tail_key = storage_key_for_map("Paraclear_perpetual_asset_balance_tail", account.0);
    state.set_storage(contract, tail_key, market);

    let base = storage_key_for_map2("Paraclear_perpetual_asset_balance", account.0, market);
    state.set_storage(contract, base, market);
    state.set_storage(contract, storage_key_with_offset(base, 1), i128_to_felt(amount));
    state.set_storage(contract, storage_key_with_offset(base, 2), i128_to_felt(cost));
    state.set_storage(contract, storage_key_with_offset(base, 3), i128_to_felt(cached_funding));
    state.set_storage(contract, storage_key_with_offset(base, 4), felt(0));
    state.set_storage(contract, storage_key_with_offset(base, 5), felt(0));
}

fn build_trade(
    maker: ContractAddress,
    taker: ContractAddress,
    market: starknet_types_core::felt::Felt,
    maker_side: starknet_types_core::felt::Felt,
    taker_side: starknet_types_core::felt::Felt,
    size: i128,
    price: i128,
    maker_reduce_only: bool,
    taker_reduce_only: bool,
    maker_category: OrderCategory,
    taker_category: OrderCategory,
) -> TradeRequestV3 {
    let maker_order = OrderV3 {
        account: maker,
        market,
        side: maker_side,
        orderType: felt(0),
        size: i128_to_felt(size),
        price: i128_to_felt(price),
        signature_timestamp: felt(10),
        is_reduce_only: maker_reduce_only,
        order_category: fee_v2_category(maker_category),
    };
    let taker_order = OrderV3 {
        account: taker,
        market,
        side: taker_side,
        orderType: felt(0),
        size: i128_to_felt(size),
        price: i128_to_felt(price),
        signature_timestamp: felt(10),
        is_reduce_only: taker_reduce_only,
        order_category: fee_v2_category(taker_category),
    };
    TradeRequestV3 {
        id: felt(0x1),
        size: i128_to_felt(size),
        price: i128_to_felt(price),
        traded_at: felt(100),
        maker_order,
        taker_order,
    }
}

fn fee_v2_category(category: OrderCategory) -> OrderCategory {
    match category {
        OrderCategory::Unspecified => {
            OrderCategory::Dynamic(FeeWithCapRequest { fee: felt(0), fee_cap: felt(0), fee_floor: felt(0) })
        }
        category => category,
    }
}

fn events_with_selector(
    events: &[crate::types::Event],
    selector: starknet_types_core::felt::Felt,
) -> Vec<&crate::types::Event> {
    events.iter().filter(|event| event.keys.first() == Some(&selector)).collect()
}

#[test]
fn test_settle_perp_success_long() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2000);
    let assets_manager = addr(0x2001);
    let oracle = addr(0x2002);
    let market = felt(0xabc);
    let settlement_token = addr(0x2003);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2101);
    let taker = addr(0x2102);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
}

#[test]
fn test_settle_perp_success_short() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2010);
    let assets_manager = addr(0x2011);
    let oracle = addr(0x2012);
    let market = felt(0xabc);
    let settlement_token = addr(0x2013);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2111);
    let taker = addr(0x2112);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
}

#[test]
fn test_settle_perp_reduce_only_rejects_increase() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2020);
    let assets_manager = addr(0x2021);
    let oracle = addr(0x2022);
    let market = felt(0xabc);
    let settlement_token = addr(0x2023);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2121);
    let taker = addr(0x2122);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        true,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events.len(), 1);
    let fail = &result.call_result.events[0];
    assert_eq!(fail.keys, vec![event_selector("SettleTradeFailedV3")]);
    assert_eq!(fail.data[0], felt(1006));
}

#[test]
fn test_settle_perp_maker_risky() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2030);
    let assets_manager = addr(0x2031);
    let oracle = addr(0x2032);
    let market = felt(0xabc);
    let settlement_token = addr(0x2033);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        10_000_000,
        false,
    );

    let maker = addr(0x2131);
    let taker = addr(0x2132);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 0);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events.len(), 1);
    let fail = &result.call_result.events[0];
    assert_eq!(fail.keys, vec![event_selector("SettleTradeFailedV3")]);
    assert_eq!(fail.data[0], felt(1001));
    assert_eq!(fail.data[1], short_str("Trade: Too risky for maker"));
}

#[test]
fn test_settle_perp_taker_risky() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2040);
    let assets_manager = addr(0x2041);
    let oracle = addr(0x2042);
    let market = felt(0xabc);
    let settlement_token = addr(0x2043);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        10_000_000,
        false,
    );

    let maker = addr(0x2141);
    let taker = addr(0x2142);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 50_000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 0);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events.len(), 1);
    let fail = &result.call_result.events[0];
    assert_eq!(fail.keys, vec![event_selector("SettleTradeFailedV3")]);
    assert_eq!(fail.data[0], felt(1001));
    assert_eq!(fail.data[1], short_str("Trade: Too risky for taker"));
}

#[test]
fn test_settle_perp_fee_v2_path() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2050);
    let assets_manager = addr(0x2051);
    let oracle = addr(0x2052);
    let market = felt(0xabc);
    let settlement_token = addr(0x2053);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2151);
    let taker = addr(0x2152);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    set_assets_manager_market_fee_config(
        &mut state,
        assets_manager,
        market,
        1_000_000,
        2_000_000,
        1_000_000,
        2_000_000,
        1_000_000,
        2_000_000,
        2_000_000,
    );

    set_account_fee_rate_future(&mut state, contract, maker, 3_000_000, 3_000_000);
    set_account_fee_rate_future(&mut state, contract, taker, 4_000_000, 4_000_000);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::API,
        OrderCategory::API,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    let fee_events = events_with_selector(&result.call_result.events, event_selector("FeeV2"));
    assert_eq!(fee_events.len(), 2);

    let mut ctx = crate::ExecutionContext::new();
    let maker_fee = paraclear::base_fee_perp_for_test(&mut ctx, &state, contract, &trade, true).expect("maker fee");
    let taker_fee = paraclear::base_fee_perp_for_test(&mut ctx, &state, contract, &trade, false).expect("taker fee");

    let maker_event = fee_events.iter().find(|event| event.data[0] == maker.0).expect("maker fee event");
    let taker_event = fee_events.iter().find(|event| event.data[0] == taker.0).expect("taker fee event");
    assert_eq!(maker_event.data[1], i128_to_felt(maker_fee));
    assert_eq!(taker_event.data[1], i128_to_felt(taker_fee));
    assert_eq!(maker_event.data[2], settlement_token.0);
    assert_eq!(taker_event.data[2], settlement_token.0);
}

#[test]
fn test_settle_perp_fee_v1_path() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2060);
    let assets_manager = addr(0x2061);
    let oracle = addr(0x2062);
    let market = felt(0xabc);
    let settlement_token = addr(0x2063);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2161);
    let taker = addr(0x2162);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    set_account_fee_rate_future(&mut state, contract, maker, 3_000_000, 3_000_000);
    set_account_fee_rate_future(&mut state, contract, taker, 4_000_000, 4_000_000);

    let mut trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );
    trade.maker_order.order_category = OrderCategory::Unspecified;
    trade.taker_order.order_category = OrderCategory::Unspecified;

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert!(result.call_result.failed);
    assert_eq!(result.revert_error.as_deref(), Some("Trade must support fee v2"));
}

#[test]
fn test_settle_perp_option_asset_kind() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2070);
    let assets_manager = addr(0x2071);
    let oracle = addr(0x2072);
    let market = felt(0xabc);
    let settlement_token = addr(0x2073);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        true,
    );

    let maker = addr(0x2171);
    let taker = addr(0x2172);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
}

#[test]
fn test_settle_perp_emits_perpetual_balance_update() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2080);
    let assets_manager = addr(0x2081);
    let oracle = addr(0x2082);
    let market = felt(0xabc);
    let settlement_token = addr(0x2083);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2181);
    let taker = addr(0x2182);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    let events = events_with_selector(&result.call_result.events, event_selector("PerpetualAssetBalanceUpdateV3"));
    assert_eq!(events.len(), 2);
}

#[test]
fn test_settle_perp_emits_trade_settled() {
    let mut state = MockStateReader::new();
    let contract = addr(0x2090);
    let assets_manager = addr(0x2091);
    let oracle = addr(0x2092);
    let market = felt(0xabc);
    let settlement_token = addr(0x2093);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x2191);
    let taker = addr(0x2192);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    let trade_events = events_with_selector(&result.call_result.events, event_selector("TradeSettled"));
    assert_eq!(trade_events.len(), 1);
}

#[test]
fn test_settle_perp_emits_fee_events() {
    let mut state = MockStateReader::new();
    let contract = addr(0x20a0);
    let assets_manager = addr(0x20a1);
    let oracle = addr(0x20a2);
    let market = felt(0xabc);
    let settlement_token = addr(0x20a3);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        0,
        0,
        false,
    );

    let maker = addr(0x21a1);
    let taker = addr(0x21a2);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);
    set_account_fee_rate_future(&mut state, contract, maker, 1_000_000, 1_000_000);
    set_account_fee_rate_future(&mut state, contract, taker, 1_000_000, 1_000_000);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    let fee_events = events_with_selector(&result.call_result.events, event_selector("FeeV2"));
    assert_eq!(fee_events.len(), 2);
}

#[test]
fn test_settle_perp_funding_applied() {
    let mut state = MockStateReader::new();
    let contract = addr(0x20b0);
    let assets_manager = addr(0x20b1);
    let oracle = addr(0x20b2);
    let market = felt(0xabc);
    let settlement_token = addr(0x20b3);
    let settlement_name = short_str("USDC");

    setup_perp_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        settlement_token,
        settlement_name,
        100 * SCALE,
        1 * SCALE,
        2 * SCALE,
        0,
        false,
    );

    let maker = addr(0x21b1);
    let taker = addr(0x21b2);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);

    set_perp_balance(&mut state, contract, maker, market, 10 * SCALE, 0, 1 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        100 * SCALE,
        false,
        false,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    let events = events_with_selector(&result.call_result.events, event_selector("PerpetualAssetBalanceUpdateV3"));
    assert_eq!(events.len(), 2);
    let maker_event = events.iter().find(|event| event.data[0] == maker.0).expect("maker event");
    assert_ne!(maker_event.data[8], felt(0));
}
