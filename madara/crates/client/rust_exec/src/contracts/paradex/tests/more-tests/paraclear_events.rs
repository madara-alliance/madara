use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::{event_selector, function_selector};
use crate::types::{ContractAddress, Event};

use starknet_types_core::felt::Felt;

use super::super::fixtures::{
    addr, felt, i128_to_felt, set_oracle_latest_tick_data, set_paraclear_dependencies, set_settlement_token,
    set_spot_asset_direct, set_token_balance, set_token_balance_amount_only, set_token_name, short_str, SCALE,
};

#[allow(clippy::too_many_arguments)]
fn setup_spot_env(
    state: &mut MockStateReader,
    contract: ContractAddress,
    assets_manager: ContractAddress,
    oracle: ContractAddress,
    market: Felt,
    base_token: ContractAddress,
    base_name: Felt,
    settlement_token: ContractAddress,
    settlement_name: Felt,
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

#[allow(clippy::too_many_arguments)]
fn build_trade(
    maker: ContractAddress,
    taker: ContractAddress,
    market: Felt,
    maker_side: Felt,
    taker_side: Felt,
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

fn event_selectors(events: &[Event]) -> Vec<Felt> {
    events.iter().map(|event| event.keys.first().copied().unwrap_or(Felt::ZERO)).collect()
}

#[test]
fn test_spot_event_ordering_basic() {
    let mut state = MockStateReader::new();
    let contract = addr(0x5000);
    let assets_manager = addr(0x5001);
    let oracle = addr(0x5002);
    let market = felt(0xabc);
    let base_token = addr(0x5003);
    let settlement_token = addr(0x5004);
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
        SCALE,
    );

    let maker = addr(0x6001);
    let taker = addr(0x6002);
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

    let selectors = event_selectors(&result.call_result.events);
    let expected = vec![
        event_selector("Fee"),
        event_selector("Fee"),
        event_selector("TokenAssetBalanceUpdate"),
        event_selector("TokenAssetBalanceUpdate"),
        event_selector("TokenAssetBalanceUpdate"),
        event_selector("TokenAssetBalanceUpdate"),
        event_selector("TradeSettled"),
    ];
    assert_eq!(selectors, expected);
}
