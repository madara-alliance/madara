use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_layout;
use crate::contracts::paradex::schema::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use crate::state::mock::MockStateReader;
use crate::storage::{storage_key_for_map, storage_key_for_map2, storage_key_with_offset};

use super::fixtures::{addr, felt, set_storage};

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

fn set_token_list(
    state: &mut MockStateReader,
    contract: crate::types::ContractAddress,
    account: crate::types::ContractAddress,
    tokens: &[u64],
) {
    if tokens.is_empty() {
        return;
    }
    let tail_key = storage_key_for_map("Paraclear_token_asset_balance_tail", account.0);
    set_storage(state, contract, tail_key, felt(tokens[0]));
    for (idx, token) in tokens.iter().enumerate() {
        let base = storage_key_for_map2("Paraclear_token_asset_balance", account.0, felt(*token));
        set_storage(state, contract, base, felt(*token));
        let next = tokens.get(idx + 1).copied().unwrap_or(0);
        set_storage(state, contract, storage_key_with_offset(base, 2), felt(next));
    }
}

fn set_perp_list(
    state: &mut MockStateReader,
    contract: crate::types::ContractAddress,
    account: crate::types::ContractAddress,
    markets: &[u64],
) {
    if markets.is_empty() {
        return;
    }
    let tail_key = paraclear::perp_map_key_for_test(0, "Paraclear_perpetual_asset_balance_tail", account.0);
    set_storage(state, contract, tail_key, felt(markets[0]));
    let mut ctx = crate::ExecutionContext::new();
    for (idx, market) in markets.iter().enumerate() {
        let base = paraclear::resolve_perp_balance_base_for_test(state, &mut ctx, contract, account, felt(*market))
            .expect("base");
        set_storage(state, contract, base, felt(*market));
        let next = markets.get(idx + 1).copied().unwrap_or(0);
        set_storage(state, contract, storage_key_with_offset(base, 4), felt(next));
    }
}

#[test]
fn test_settle_trade_v3_maker_exceeds_max_assets() {
    let mut state = MockStateReader::new();
    let contract = addr(0xA01);
    let trade = make_trade(0x20, 0x21, false, false);

    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(1));

    set_token_list(&mut state, contract, trade.maker_order.account, &[0x100, 0x101]);

    let mut ctx = crate::ExecutionContext::new();
    let err = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade).unwrap_err();
    assert!(format!("{err}").contains("maker too many assets"));
}

#[test]
fn test_settle_trade_v3_taker_exceeds_max_assets() {
    let mut state = MockStateReader::new();
    let contract = addr(0xA02);
    let trade = make_trade(0x30, 0x31, false, false);

    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(1));

    set_perp_list(&mut state, contract, trade.taker_order.account, &[0x200, 0x201]);

    let mut ctx = crate::ExecutionContext::new();
    let err = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade).unwrap_err();
    assert!(format!("{err}").contains("taker too many assets"));
}

#[test]
fn test_settle_trade_v3_reduce_only_bypasses_max_assets_check() {
    let mut state = MockStateReader::new();
    let contract = addr(0xA03);
    let trade = make_trade(0x40, 0x41, true, false);

    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(1));

    set_token_list(&mut state, contract, trade.maker_order.account, &[0x300, 0x301]);

    let mut ctx = crate::ExecutionContext::new();
    let result = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade);
    assert!(result.is_ok());
}
