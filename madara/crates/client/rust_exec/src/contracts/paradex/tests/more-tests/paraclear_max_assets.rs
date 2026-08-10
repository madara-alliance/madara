use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_layout;
use crate::contracts::paradex::schema::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::storage_key_with_offset;

use super::super::fixtures::{addr, felt, set_storage};

fn make_trade(maker: u64, taker: u64, reduce_only_maker: bool, reduce_only_taker: bool) -> TradeRequestV3 {
    let maker_order = OrderV3 {
        account: addr(maker),
        market: felt(0xabc),
        side: felt(1),
        orderType: felt(0),
        size: felt(5),
        price: felt(100),
        signature_timestamp: felt(10),
        is_reduce_only: reduce_only_maker,
        order_category: OrderCategory::Unspecified,
    };
    let taker_order = OrderV3 {
        account: addr(taker),
        market: maker_order.market,
        side: felt(2),
        orderType: felt(0),
        size: felt(5),
        price: felt(100),
        signature_timestamp: felt(10),
        is_reduce_only: reduce_only_taker,
        order_category: OrderCategory::Unspecified,
    };
    TradeRequestV3 { id: felt(0x1), size: felt(5), price: felt(100), traded_at: felt(10), maker_order, taker_order }
}

#[test]
fn test_enforce_max_assets_default_fallback() {
    let mut state = MockStateReader::new();
    let contract = addr(0xA00);
    let trade = make_trade(0x10, 0x11, false, false);

    // max_assets_per_account = 0 => default 150
    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(0));

    let mut ctx = crate::ExecutionContext::new();
    let result = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade);
    assert!(result.is_ok());
}
