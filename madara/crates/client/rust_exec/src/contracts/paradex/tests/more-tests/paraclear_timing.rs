#![allow(clippy::too_many_arguments)]

use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::core::state::mock::MockStateReader;
use crate::core::storage::function_selector;
use crate::core::types::ContractAddress;

use starknet_types_core::felt::Felt;

use super::super::fixtures::{addr, felt, i128_to_felt, SCALE};

fn build_trade(
    maker: ContractAddress,
    taker: ContractAddress,
    market: Felt,
    maker_side: Felt,
    taker_side: Felt,
    size: i128,
    price: i128,
    traded_at: Felt,
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
        order_category: OrderCategory::Unspecified,
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
        order_category: OrderCategory::Unspecified,
    };
    TradeRequestV3 {
        id: felt(0x1),
        size: i128_to_felt(size),
        price: i128_to_felt(price),
        traded_at,
        maker_order,
        taker_order,
    }
}

#[test]
fn test_execute_with_timestamp_future_trade_fails() {
    let state = MockStateReader::new();
    let contract = addr(0x7000);
    let maker = addr(0x7001);
    let taker = addr(0x7002);
    let market = felt(0xabc);

    let trade = build_trade(maker, taker, market, felt(2), felt(1), 2 * SCALE, 100 * SCALE, felt(5000));

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result =
        paraclear::execute_with_timestamp(&state, contract, selector, &calldata, addr(0x999), 1000).expect("execute");

    assert!(result.call_result.failed);
    assert_eq!(result.revert_error.as_deref(), Some("Trade: traded_at from future"));
}
