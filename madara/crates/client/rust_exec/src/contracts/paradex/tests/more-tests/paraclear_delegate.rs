use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_layout;
use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::{event_selector, function_selector};
use starknet_types_core::felt::Felt;

use super::super::fixtures::{addr, felt, set_storage};

fn sample_trade(market: Felt) -> TradeRequestV3 {
    let maker_order = OrderV3 {
        account: addr(0x10),
        market,
        side: felt(1),
        orderType: felt(0),
        size: felt(5),
        price: felt(100),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: OrderCategory::Unspecified,
    };
    let taker_order = OrderV3 {
        account: addr(0x11),
        market,
        side: felt(2),
        orderType: felt(0),
        size: felt(5),
        price: felt(100),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: OrderCategory::Unspecified,
    };
    TradeRequestV3 { id: felt(0x1), size: felt(5), price: felt(100), traded_at: felt(10), maker_order, taker_order }
}

#[test]
fn test_market_delegate_none_runs_local() {
    let mut state = MockStateReader::new();
    let contract = addr(0x700);
    let market = felt(0xabc);

    // Explicitly set delegate to zero.
    let key = paraclear_layout::Paraclear_market_delegate_key(market);
    set_storage(&mut state, contract, key, felt(0));

    let trade = sample_trade(market);
    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");

    // Asset kind unsupported -> returns Ok with SettleTradeFailedV3 (no delegate errors).
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events[0].keys, vec![event_selector("SettleTradeFailedV3")]);
}

#[test]
fn test_market_delegate_missing_class_hash_is_ignored() {
    let mut state = MockStateReader::new();
    let contract = addr(0x701);
    let market = felt(0xabc);
    let delegate = addr(0x800);

    let key = paraclear_layout::Paraclear_market_delegate_key(market);
    set_storage(&mut state, contract, key, delegate.0);

    let trade = sample_trade(market);
    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");

    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events[0].keys, vec![event_selector("SettleTradeFailedV3")]);
}

#[test]
fn test_market_delegate_unsupported_contract_is_ignored() {
    let mut state = MockStateReader::new();
    let contract = addr(0x702);
    let market = felt(0xabc);
    let delegate = addr(0x801);

    let key = paraclear_layout::Paraclear_market_delegate_key(market);
    set_storage(&mut state, contract, key, delegate.0);

    // Set class hash for delegate to make sure the local implementation ignores it.
    state.set_class_hash(delegate, felt(0xdeadbeef));

    let trade = sample_trade(market);
    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");

    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");
    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events[0].keys, vec![event_selector("SettleTradeFailedV3")]);
}
