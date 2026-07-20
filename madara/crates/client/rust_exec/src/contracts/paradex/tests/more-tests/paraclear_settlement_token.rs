use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_layout;
use crate::state::mock::MockStateReader;
use crate::storage::{short_string_to_felt, storage_key_with_offset};

use super::super::fixtures::{addr, felt, set_oracle_latest_tick_data, set_storage, short_str};

#[test]
fn test_read_settlement_token_address() {
    let mut state = MockStateReader::new();
    let contract = addr(0x600);
    let base = *paraclear_layout::PARACLEAR_SETTLEMENT_TOKEN_ASSET_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), addr(0x1).0);

    let mut ctx = crate::ExecutionContext::new();
    let first = paraclear::read_settlement_token_address_for_test(&mut ctx, &state, contract).expect("address");
    assert_eq!(first, addr(0x1));

    // Change underlying storage after cache.
    set_storage(&mut state, contract, storage_key_with_offset(base, 4), addr(0x2).0);
    let mut fresh_ctx = crate::ExecutionContext::new();
    let second = paraclear::read_settlement_token_address_for_test(&mut fresh_ctx, &state, contract).expect("address");
    assert_eq!(second, addr(0x2));
}

#[test]
fn test_read_settlement_token_name() {
    let mut state = MockStateReader::new();
    let contract = addr(0x601);
    let base = *paraclear_layout::PARACLEAR_SETTLEMENT_TOKEN_ASSET_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 5), short_str("USDC"));

    let mut ctx = crate::ExecutionContext::new();
    let first = paraclear::read_settlement_token_name_for_test(&mut ctx, &state, contract).expect("name");
    assert_eq!(first, short_str("USDC"));

    set_storage(&mut state, contract, storage_key_with_offset(base, 5), short_str("WETH"));
    let mut fresh_ctx = crate::ExecutionContext::new();
    let second = paraclear::read_settlement_token_name_for_test(&mut fresh_ctx, &state, contract).expect("name");
    assert_eq!(second, short_str("WETH"));
}

#[test]
fn test_read_settlement_token_price() {
    let mut state = MockStateReader::new();
    let oracle = addr(0x602);
    let usdc = short_string_to_felt("USDC");

    set_oracle_latest_tick_data(&mut state, oracle, usdc, usdc, felt(0x100), felt(8));

    let mut ctx = crate::ExecutionContext::new();
    let first = paraclear::read_settlement_token_price_for_test(&mut ctx, &state, oracle, usdc).expect("price");
    assert_eq!(first, felt(0x100));

    // Change underlying storage after cache.
    set_oracle_latest_tick_data(&mut state, oracle, usdc, usdc, felt(0x200), felt(8));
    let mut fresh_ctx = crate::ExecutionContext::new();
    let second = paraclear::read_settlement_token_price_for_test(&mut fresh_ctx, &state, oracle, usdc).expect("price");
    assert_eq!(second, felt(0x200));
}

#[test]
fn test_read_settlement_token_price_zero_errors() {
    use crate::contracts::paradex_codegen::paraclear_types::{
        FeeWithCapRequest, OrderCategory, OrderV3, TradeRequestV3,
    };

    let mut state = MockStateReader::new();
    let contract = addr(0x603);
    let assets_manager = addr(0x604);
    let oracle = addr(0x605);
    let usdc = short_string_to_felt("USDC");

    // Wire dependencies.
    set_storage(&mut state, contract, *paraclear_layout::ASSETS_MANAGER_BASE, assets_manager.0);
    set_storage(&mut state, contract, *paraclear_layout::PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE, oracle.0);

    // Make asset kind SPOT for the market.
    let market = felt(0xabc);
    super::super::fixtures::set_spot_asset_direct(
        &mut state,
        assets_manager,
        market,
        addr(0x777),
        short_str("AAA"),
        felt(0x88),
    );

    // Settlement token price is zero.
    set_oracle_latest_tick_data(&mut state, oracle, usdc, usdc, felt(0), felt(8));

    let maker_order = OrderV3 {
        account: addr(0x10),
        market,
        side: felt(1),
        orderType: felt(0),
        size: felt(5),
        price: felt(100),
        signature_timestamp: felt(10),
        is_reduce_only: false,
        order_category: OrderCategory::Dynamic(FeeWithCapRequest {
            fee: felt(0),
            fee_cap: felt(0),
            fee_floor: felt(0),
        }),
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
        order_category: OrderCategory::Dynamic(FeeWithCapRequest {
            fee: felt(0),
            fee_cap: felt(0),
            fee_floor: felt(0),
        }),
    };
    let trade = TradeRequestV3 {
        id: felt(0x1),
        size: felt(5),
        price: felt(100),
        traded_at: felt(10),
        maker_order,
        taker_order,
    };

    let calldata = super::super::fixtures::encode_trade_request_v3_for_test(&trade);
    let selector = crate::storage::function_selector("settle_trade_v3");
    let err = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).unwrap_err();
    assert!(format!("{err}").contains("settlement_token_price is zero"));
}

#[test]
fn test_read_contract_address() {
    let mut state = MockStateReader::new();
    let contract = addr(0x604);
    let key = *paraclear_layout::ASSETS_MANAGER_BASE;
    set_storage(&mut state, contract, key, addr(0xaa).0);

    let mut ctx = crate::ExecutionContext::new();
    let first = paraclear::read_contract_address_for_test(&mut ctx, &state, contract, key).expect("address");
    assert_eq!(first, addr(0xaa));

    set_storage(&mut state, contract, key, addr(0xbb).0);
    let mut fresh_ctx = crate::ExecutionContext::new();
    let second = paraclear::read_contract_address_for_test(&mut fresh_ctx, &state, contract, key).expect("address");
    assert_eq!(second, addr(0xbb));
}

#[test]
fn test_resolve_market_delegate_address_reads_storage() {
    let mut state = MockStateReader::new();
    let contract = addr(0x605);
    let market = felt(0x12);

    let key = paraclear_layout::Paraclear_market_delegate_key(market);
    set_storage(&mut state, contract, key, addr(0x999).0);

    let mut ctx = crate::ExecutionContext::new();
    let delegate =
        paraclear::resolve_market_delegate_address_for_test(&mut ctx, &state, contract, market).expect("delegate");
    assert_eq!(delegate, Some(addr(0x999)));
}
