use crate::contracts::paradex::{assets_manager, paraclear};
use crate::state::mock::MockStateReader;

use super::fixtures::{addr, felt, set_future_asset_direct, set_option_asset_direct, set_spot_asset_direct, short_str};

#[test]
fn test_get_asset_kind_future() {
    let mut state = MockStateReader::new();
    let paraclear_contract = addr(0x500);
    let assets_manager_contract = addr(0x501);
    let market = felt(0x10);

    set_future_asset_direct(
        &mut state,
        assets_manager_contract,
        market,
        felt(0x1),
        felt(0x2),
        felt(0x3),
        felt(0x4),
        felt(0x5),
        felt(0x6),
        felt(0x7),
    );

    let mut ctx = crate::ExecutionContext::new();
    let kind =
        paraclear::get_asset_kind_for_test(&mut ctx, &state, paraclear_contract, assets_manager_contract, market)
            .expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_future());
}

#[test]
fn test_get_asset_kind_option() {
    let mut state = MockStateReader::new();
    let paraclear_contract = addr(0x510);
    let assets_manager_contract = addr(0x511);
    let market = felt(0x11);

    set_option_asset_direct(
        &mut state,
        assets_manager_contract,
        market,
        felt(0x11),
        felt(0x12),
        felt(0x13),
        felt(0x14),
        felt(0x15),
    );

    let mut ctx = crate::ExecutionContext::new();
    let kind =
        paraclear::get_asset_kind_for_test(&mut ctx, &state, paraclear_contract, assets_manager_contract, market)
            .expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_option());
}

#[test]
fn test_get_asset_kind_spot() {
    let mut state = MockStateReader::new();
    let paraclear_contract = addr(0x520);
    let assets_manager_contract = addr(0x521);
    let market = felt(0x12);

    set_spot_asset_direct(&mut state, assets_manager_contract, market, addr(0x901), short_str("AAA"), felt(0x88));

    let mut ctx = crate::ExecutionContext::new();
    let kind =
        paraclear::get_asset_kind_for_test(&mut ctx, &state, paraclear_contract, assets_manager_contract, market)
            .expect("kind");
    assert_eq!(kind, assets_manager::asset_kind_spot());
}

#[test]
fn test_get_asset_kind_unsupported() {
    let state = MockStateReader::new();
    let paraclear_contract = addr(0x530);
    let assets_manager_contract = addr(0x531);
    let market = felt(0x13);

    let mut ctx = crate::ExecutionContext::new();
    let kind =
        paraclear::get_asset_kind_for_test(&mut ctx, &state, paraclear_contract, assets_manager_contract, market)
            .expect("kind");
    assert_eq!(kind, assets_manager::ASSET_KIND_UNSUPPORTED);
}

#[test]
fn test_get_asset_kind_zero_market() {
    let state = MockStateReader::new();
    let paraclear_contract = addr(0x540);
    let assets_manager_contract = addr(0x541);

    let mut ctx = crate::ExecutionContext::new();
    let kind =
        paraclear::get_asset_kind_for_test(&mut ctx, &state, paraclear_contract, assets_manager_contract, felt(0))
            .expect("kind");
    assert_eq!(kind, assets_manager::ASSET_KIND_UNSUPPORTED);
}

#[test]
fn test_settle_trade_v3_not_supported_market() {
    use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
    use crate::storage::event_selector;

    let state = MockStateReader::new();
    let contract = addr(0x550);
    let caller = addr(0x551);

    let maker_order = OrderV3 {
        account: addr(0x10),
        market: felt(0xabc),
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
        market: maker_order.market,
        side: felt(2),
        orderType: felt(0),
        size: felt(5),
        price: felt(100),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: OrderCategory::Unspecified,
    };
    let trade = TradeRequestV3 {
        id: felt(0x1),
        size: felt(5),
        price: felt(100),
        traded_at: felt(10),
        maker_order,
        taker_order,
    };

    let calldata = super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = crate::storage::function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, caller).expect("execute");

    assert_eq!(result.call_result.retdata, vec![felt(0)]);
    assert_eq!(result.call_result.events.len(), 1);
    assert_eq!(result.call_result.events[0].keys, vec![event_selector("SettleTradeFailedV3")]);
}
