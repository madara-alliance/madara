use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_types::OrderCategory;
use crate::state::mock::MockStateReader;
use crate::storage::storage_key_with_offset;

use super::super::fixtures::{
    addr, dynamic_fee_request, felt, sample_trade, set_account_fee_rate_spot, set_future_asset_direct,
    set_paraclear_dependencies, set_storage,
};

#[test]
fn test_trade_support_fee_v2_dynamic_true() {
    let trade = sample_trade(OrderCategory::Dynamic(dynamic_fee_request()), OrderCategory::API);
    assert!(paraclear::trade_support_fee_v2_for_test(&trade));
}

#[test]
fn test_apply_referral_discount_and_commission() {
    let fee = 100_000_000i128; // 1.0 scaled
    let referrer = addr(0x1234);
    let fee_discount = 50_000_000i128; // 0.5
    let fee_commission = 10_000_000i128; // ignored by discount function

    let discounted =
        paraclear::apply_referral_discount_for_test(fee, referrer, fee_discount, fee_commission).expect("discount");
    assert_eq!(discounted, 50_000_000i128);

    let no_referrer =
        paraclear::apply_referral_discount_for_test(fee, addr(0), fee_discount, fee_commission).expect("discount");
    assert_eq!(no_referrer, fee);
}

#[test]
fn test_fee_rate_v2_for_order_api() {
    let mut state = MockStateReader::new();
    let contract = addr(0x800);
    let market = felt(0xabc);
    let base = paraclear::perp_map_key_for_test(0, "perpetual_future_market_fee_config_v2", market);

    set_storage(&mut state, contract, base, felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(10));
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), felt(11));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), felt(12));
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(13));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(14));
    set_storage(&mut state, contract, storage_key_with_offset(base, 6), felt(15));

    let mut ctx = crate::ExecutionContext::new();
    let maker =
        paraclear::fee_rate_v2_for_order_for_test(&mut ctx, &state, contract, market, &OrderCategory::API, true, 0)
            .expect("rate");
    let taker =
        paraclear::fee_rate_v2_for_order_for_test(&mut ctx, &state, contract, market, &OrderCategory::API, false, 0)
            .expect("rate");

    assert_eq!(maker, Some(10));
    assert_eq!(taker, Some(11));
}

#[test]
fn test_fee_rate_v2_for_order_rpi() {
    let mut state = MockStateReader::new();
    let contract = addr(0x801);
    let market = felt(0xabc);
    let base = paraclear::perp_map_key_for_test(0, "perpetual_future_market_fee_config_v2", market);

    set_storage(&mut state, contract, base, felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(10));
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), felt(11));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), felt(12));
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(13));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(14));
    set_storage(&mut state, contract, storage_key_with_offset(base, 6), felt(15));

    let mut ctx = crate::ExecutionContext::new();
    let maker =
        paraclear::fee_rate_v2_for_order_for_test(&mut ctx, &state, contract, market, &OrderCategory::RPI, true, 0)
            .expect("rate");
    let taker =
        paraclear::fee_rate_v2_for_order_for_test(&mut ctx, &state, contract, market, &OrderCategory::RPI, false, 0)
            .expect("rate");

    assert_eq!(maker, Some(12));
    assert_eq!(taker, Some(13));
}

#[test]
fn test_fee_rate_v2_for_order_interactive() {
    let mut state = MockStateReader::new();
    let contract = addr(0x802);
    let market = felt(0xabc);
    let base = paraclear::perp_map_key_for_test(0, "perpetual_future_market_fee_config_v2", market);

    set_storage(&mut state, contract, base, felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(10));
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), felt(11));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), felt(12));
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(13));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(14));
    set_storage(&mut state, contract, storage_key_with_offset(base, 6), felt(15));

    let mut ctx = crate::ExecutionContext::new();
    let maker = paraclear::fee_rate_v2_for_order_for_test(
        &mut ctx,
        &state,
        contract,
        market,
        &OrderCategory::Interactive,
        true,
        0,
    )
    .expect("rate");
    let taker = paraclear::fee_rate_v2_for_order_for_test(
        &mut ctx,
        &state,
        contract,
        market,
        &OrderCategory::Interactive,
        false,
        0,
    )
    .expect("rate");

    assert_eq!(maker, Some(14));
    assert_eq!(taker, Some(15));
}

#[test]
fn test_fee_rate_v2_for_order_dynamic() {
    let mut state = MockStateReader::new();
    let contract = addr(0x803);
    let market = felt(0xabc);
    let base = paraclear::perp_map_key_for_test(0, "perpetual_future_market_fee_config_v2", market);

    set_storage(&mut state, contract, base, felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(10));
    set_storage(&mut state, contract, storage_key_with_offset(base, 2), felt(11));
    set_storage(&mut state, contract, storage_key_with_offset(base, 3), felt(12));
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), felt(13));
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), felt(14));
    set_storage(&mut state, contract, storage_key_with_offset(base, 6), felt(15));

    let mut ctx = crate::ExecutionContext::new();
    let category = OrderCategory::Dynamic(super::super::fixtures::dynamic_fee_request());
    let rate = paraclear::fee_rate_v2_for_order_for_test(&mut ctx, &state, contract, market, &category, true, 0)
        .expect("rate");
    assert_eq!(rate, Some(7));
}

const SCALE: i128 = 100_000_000;

fn trade_with_size_price(
    size: i128,
    price: i128,
) -> crate::contracts::paradex::schema::paraclear_types::TradeRequestV3 {
    use crate::contracts::paradex::schema::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
    let maker_order = OrderV3 {
        account: addr(0x900),
        market: felt(0xabc),
        side: felt(1),
        orderType: felt(0),
        size: felt(size as u64),
        price: felt(price as u64),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: OrderCategory::Unspecified,
    };
    let taker_order = OrderV3 {
        account: addr(0x901),
        market: maker_order.market,
        side: felt(2),
        orderType: felt(0),
        size: felt(size as u64),
        price: felt(price as u64),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: OrderCategory::Unspecified,
    };
    TradeRequestV3 {
        id: felt(0x1),
        size: felt(size as u64),
        price: felt(price as u64),
        traded_at: felt(10),
        maker_order,
        taker_order,
    }
}

#[test]
fn test_base_fee_spot_maker() {
    let mut state = MockStateReader::new();
    let contract = addr(0x9000);

    let trade = trade_with_size_price(2 * SCALE, 3 * SCALE);
    set_account_fee_rate_spot(&mut state, contract, trade.maker_order.account, 4 * SCALE, 5 * SCALE);

    let mut ctx = crate::ExecutionContext::new();
    let fee = paraclear::base_fee_spot_for_test(&mut ctx, &state, contract, &trade, true).expect("fee");
    assert_eq!(fee, 24 * SCALE);
}

#[test]
fn test_base_fee_spot_taker() {
    let mut state = MockStateReader::new();
    let contract = addr(0x9001);

    let trade = trade_with_size_price(2 * SCALE, 3 * SCALE);
    set_account_fee_rate_spot(&mut state, contract, trade.taker_order.account, 4 * SCALE, 5 * SCALE);

    let mut ctx = crate::ExecutionContext::new();
    let fee = paraclear::base_fee_spot_for_test(&mut ctx, &state, contract, &trade, false).expect("fee");
    assert_eq!(fee, 30 * SCALE);
}

#[test]
fn test_base_fee_perp_maker() {
    let mut state = MockStateReader::new();
    let contract = addr(0x9002);
    let assets_manager = addr(0x9004);
    let oracle = addr(0x9005);

    let trade = trade_with_size_price(2 * SCALE, 3 * SCALE);
    set_paraclear_dependencies(&mut state, contract, assets_manager, oracle);
    set_future_asset_direct(
        &mut state,
        assets_manager,
        trade.maker_order.market,
        felt(0x1),
        felt(0x2),
        felt(1),
        felt(0),
        felt(0),
        felt(0),
        felt(0),
    );
    let maker_base = crate::contracts::paradex::schema::account_component_layout::Paraclear_account_fee_rate_key(
        trade.maker_order.account.0,
    );
    set_storage(&mut state, contract, maker_base, felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(maker_base, 1), felt((6 * SCALE) as u64));
    set_storage(&mut state, contract, storage_key_with_offset(maker_base, 2), felt((7 * SCALE) as u64));

    let mut ctx = crate::ExecutionContext::new();
    let fee = paraclear::base_fee_perp_for_test(&mut ctx, &state, contract, &trade, true).expect("fee");
    assert_eq!(fee, 36 * SCALE);
}

#[test]
fn test_base_fee_perp_taker() {
    let mut state = MockStateReader::new();
    let contract = addr(0x9003);
    let assets_manager = addr(0x9006);
    let oracle = addr(0x9007);

    let trade = trade_with_size_price(2 * SCALE, 3 * SCALE);
    set_paraclear_dependencies(&mut state, contract, assets_manager, oracle);
    set_future_asset_direct(
        &mut state,
        assets_manager,
        trade.maker_order.market,
        felt(0x1),
        felt(0x2),
        felt(1),
        felt(0),
        felt(0),
        felt(0),
        felt(0),
    );
    let taker_base = crate::contracts::paradex::schema::account_component_layout::Paraclear_account_fee_rate_key(
        trade.taker_order.account.0,
    );
    set_storage(&mut state, contract, taker_base, felt(1));
    set_storage(&mut state, contract, storage_key_with_offset(taker_base, 1), felt((6 * SCALE) as u64));
    set_storage(&mut state, contract, storage_key_with_offset(taker_base, 2), felt((7 * SCALE) as u64));

    let mut ctx = crate::ExecutionContext::new();
    let fee = paraclear::base_fee_perp_for_test(&mut ctx, &state, contract, &trade, false).expect("fee");
    assert_eq!(fee, 42 * SCALE);
}
