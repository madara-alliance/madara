use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::{event_selector, function_selector};
use crate::types::ContractAddress;

use super::super::fixtures::{
    addr, felt, i128_to_felt, set_account_fee_rate_spot, set_account_referral, set_fee_share,
    set_oracle_latest_tick_data, set_paraclear_dependencies, set_settlement_token, set_spot_asset_direct,
    set_token_balance, set_token_balance_amount_only, set_token_name, short_str, SCALE,
};

fn setup_spot_env(
    state: &mut MockStateReader,
    contract: ContractAddress,
    assets_manager: ContractAddress,
    oracle: ContractAddress,
    market: starknet_types_core::felt::Felt,
    base_token: ContractAddress,
    base_name: starknet_types_core::felt::Felt,
    settlement_token: ContractAddress,
    settlement_name: starknet_types_core::felt::Felt,
    base_price: i128,
    settlement_price: i128,
) {
    set_paraclear_dependencies(state, contract, assets_manager, oracle);
    set_settlement_token(state, contract, settlement_token, settlement_name);
    set_token_name(state, contract, base_token, base_name);
    set_token_name(state, contract, settlement_token, settlement_name);
    set_spot_asset_direct(state, assets_manager, market, base_token, base_name, felt(0x0));
    set_oracle_latest_tick_data(state, oracle, base_name, base_name, i128_to_felt(base_price), felt(8));
    set_oracle_latest_tick_data(
        state,
        oracle,
        settlement_name,
        settlement_name,
        i128_to_felt(settlement_price),
        felt(8),
    );
}

fn build_trade(
    maker: ContractAddress,
    taker: ContractAddress,
    market: starknet_types_core::felt::Felt,
    maker_side: starknet_types_core::felt::Felt,
    taker_side: starknet_types_core::felt::Felt,
    size: i128,
    price: i128,
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
        is_reduce_only: false,
        order_category: maker_category,
    };
    let taker_order = OrderV3 {
        account: taker,
        market,
        side: taker_side,
        orderType: felt(0),
        size: i128_to_felt(size),
        price: i128_to_felt(price),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: taker_category,
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

fn events_with_selector<'a>(
    events: &'a [crate::types::Event],
    selector: starknet_types_core::felt::Felt,
) -> Vec<&'a crate::types::Event> {
    events.iter().filter(|event| event.keys.first() == Some(&selector)).collect()
}

#[test]
fn test_settle_spot_success_buy() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1000);
    let assets_manager = addr(0x1001);
    let oracle = addr(0x1002);
    let market = felt(0xabc);
    let base_token = addr(0x2001);
    let settlement_token = addr(0x2002);
    let base_name = short_str("AAA");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3001);
    let taker = addr(0x3002);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 500 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 2000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
    let trade_events = events_with_selector(&result.call_result.events, event_selector("TradeSettled"));
    assert_eq!(trade_events.len(), 1);
}

#[test]
fn test_settle_spot_success_sell() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1010);
    let assets_manager = addr(0x1011);
    let oracle = addr(0x1012);
    let market = felt(0xabc);
    let base_token = addr(0x2011);
    let settlement_token = addr(0x2012);
    let base_name = short_str("BBB");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3011);
    let taker = addr(0x3012);
    set_token_balance(&mut state, contract, taker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 2000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        200 * SCALE,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
}

#[test]
fn test_settle_spot_maker_insufficient_balance() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1020);
    let assets_manager = addr(0x1021);
    let oracle = addr(0x1022);
    let market = felt(0xabc);
    let base_token = addr(0x2021);
    let settlement_token = addr(0x2022);
    let base_name = short_str("CCC");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3021);
    let taker = addr(0x3022);
    set_token_balance(&mut state, contract, maker, base_token, 1 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 2000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
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
    assert_eq!(fail.data[0], felt(1008));
}

#[test]
fn test_settle_spot_taker_insufficient_balance() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1030);
    let assets_manager = addr(0x1031);
    let oracle = addr(0x1032);
    let market = felt(0xabc);
    let base_token = addr(0x2031);
    let settlement_token = addr(0x2032);
    let base_name = short_str("DDD");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3031);
    let taker = addr(0x3032);
    set_token_balance(&mut state, contract, taker, base_token, 1 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 2000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        200 * SCALE,
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
    assert_eq!(fail.data[0], felt(1009));
}

#[test]
fn test_settle_spot_maker_risky() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1040);
    let assets_manager = addr(0x1041);
    let oracle = addr(0x1042);
    let market = felt(0xabc);
    let base_token = addr(0x2041);
    let settlement_token = addr(0x2042);
    let base_name = short_str("EEE");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3041);
    let taker = addr(0x3042);
    set_token_balance(&mut state, contract, taker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 0);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(1),
        felt(2),
        5 * SCALE,
        200 * SCALE,
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
fn test_settle_spot_taker_risky() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1050);
    let assets_manager = addr(0x1051);
    let oracle = addr(0x1052);
    let market = felt(0xabc);
    let base_token = addr(0x2051);
    let settlement_token = addr(0x2052);
    let base_name = short_str("FFF");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3051);
    let taker = addr(0x3052);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 2000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 0);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
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
fn test_settle_spot_referrer_fee_share_both() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1060);
    let assets_manager = addr(0x1061);
    let oracle = addr(0x1062);
    let market = felt(0xabc);
    let base_token = addr(0x2061);
    let settlement_token = addr(0x2062);
    let base_name = short_str("GGG");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3061);
    let taker = addr(0x3062);
    let maker_referrer = addr(0x4001);
    let taker_referrer = addr(0x4002);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 500 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 2000 * SCALE);

    set_account_fee_rate_spot(&mut state, contract, maker, 1_000_000, 1_000_000);
    set_account_fee_rate_spot(&mut state, contract, taker, 1_000_000, 1_000_000);
    set_account_referral(&mut state, contract, maker, maker_referrer, 10_000_000, 0);
    set_account_referral(&mut state, contract, taker, taker_referrer, 10_000_000, 0);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
    let fee_share_events = events_with_selector(&result.call_result.events, event_selector("FeeShare"));
    assert_eq!(fee_share_events.len(), 2);
    let accounts: Vec<_> = fee_share_events.iter().map(|event| event.data[0]).collect();
    assert!(accounts.contains(&maker_referrer.0));
    assert!(accounts.contains(&taker_referrer.0));
}

#[test]
fn test_settle_spot_fee_share_account() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1070);
    let assets_manager = addr(0x1071);
    let oracle = addr(0x1072);
    let market = felt(0xabc);
    let base_token = addr(0x2071);
    let settlement_token = addr(0x2072);
    let base_name = short_str("HHH");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3071);
    let taker = addr(0x3072);
    let fee_share_account = addr(0x5001);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 500 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 2000 * SCALE);
    set_account_fee_rate_spot(&mut state, contract, maker, 1_000_000, 1_000_000);
    set_account_fee_rate_spot(&mut state, contract, taker, 1_000_000, 1_000_000);
    set_fee_share(&mut state, contract, fee_share_account, 20_000_000);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(1)]);
    let fee_share_events = events_with_selector(&result.call_result.events, event_selector("FeeShare"));
    assert_eq!(fee_share_events.len(), 1);
    assert_eq!(fee_share_events[0].data[0], fee_share_account.0);
}

#[test]
fn test_settle_spot_emits_trade_settled() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1080);
    let assets_manager = addr(0x1081);
    let oracle = addr(0x1082);
    let market = felt(0xabc);
    let base_token = addr(0x2081);
    let settlement_token = addr(0x2082);
    let base_name = short_str("III");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3081);
    let taker = addr(0x3082);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 500 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 2000 * SCALE);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
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
fn test_settle_spot_emits_fee_events() {
    let mut state = MockStateReader::new();
    let contract = addr(0x1090);
    let assets_manager = addr(0x1091);
    let oracle = addr(0x1092);
    let market = felt(0xabc);
    let base_token = addr(0x2091);
    let settlement_token = addr(0x2092);
    let base_name = short_str("JJJ");
    let settlement_name = short_str("USDC");

    setup_spot_env(
        &mut state,
        contract,
        assets_manager,
        oracle,
        market,
        base_token,
        base_name,
        settlement_token,
        settlement_name,
        2 * SCALE,
        1 * SCALE,
    );

    let maker = addr(0x3091);
    let taker = addr(0x3092);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 5000 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 5000 * SCALE);
    set_account_fee_rate_spot(&mut state, contract, maker, 1_000_000, 0);
    set_account_fee_rate_spot(&mut state, contract, taker, 0, 2_000_000);

    let trade = build_trade(
        maker,
        taker,
        market,
        felt(2),
        felt(1),
        5 * SCALE,
        200 * SCALE,
        OrderCategory::Unspecified,
        OrderCategory::Unspecified,
    );

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    let fee_events = events_with_selector(&result.call_result.events, event_selector("Fee"));
    assert_eq!(fee_events.len(), 2);

    let mut ctx = crate::ExecutionContext::new();
    let maker_fee = paraclear::base_fee_spot_for_test(&mut ctx, &state, contract, &trade, true).expect("maker fee");
    let taker_fee = paraclear::base_fee_spot_for_test(&mut ctx, &state, contract, &trade, false).expect("taker fee");

    let maker_event = fee_events.iter().find(|event| event.data[0] == maker.0).expect("maker fee event");
    let taker_event = fee_events.iter().find(|event| event.data[0] == taker.0).expect("taker fee event");

    assert_eq!(maker_event.data[1], i128_to_felt(maker_fee));
    assert_eq!(taker_event.data[1], i128_to_felt(taker_fee));
}
