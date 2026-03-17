use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::function_selector;
use crate::types::ContractAddress;

use starknet_types_core::felt::Felt;

use super::super::fixtures::{
    addr, felt, i128_to_felt, set_oracle_latest_tick_data, set_paraclear_dependencies, set_settlement_token,
    set_spot_asset_direct, set_token_balance, set_token_balance_amount_only, set_token_name, short_str, SCALE,
};

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

fn build_trade(
    maker: ContractAddress,
    taker: ContractAddress,
    market: Felt,
    maker_side: Felt,
    taker_side: Felt,
    size: i128,
    price: i128,
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
        traded_at: felt(100),
        maker_order,
        taker_order,
    }
}

#[test]
fn test_spot_trade_gas_consumed_nonzero() {
    let mut state = MockStateReader::new();
    let contract = addr(0x8000);
    let assets_manager = addr(0x8001);
    let oracle = addr(0x8002);
    let market = felt(0xabc);
    let base_token = addr(0x8003);
    let settlement_token = addr(0x8004);
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

    let maker = addr(0x8101);
    let taker = addr(0x8102);
    set_token_balance(&mut state, contract, maker, base_token, 10 * SCALE);
    set_token_balance_amount_only(&mut state, contract, maker, settlement_token, 500 * SCALE);
    set_token_balance_amount_only(&mut state, contract, taker, settlement_token, 2000 * SCALE);

    let trade = build_trade(maker, taker, market, felt(2), felt(1), 5 * SCALE, 200 * SCALE);

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    assert!(result.call_result.gas_consumed > 0);
}
