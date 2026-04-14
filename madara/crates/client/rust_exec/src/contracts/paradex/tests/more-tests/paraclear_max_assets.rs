use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_layout;
use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
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

// ── Custom asset limit tests (ported from Cairo) ───────────────────────────

#[test]
fn test_custom_max_assets_below_limit_succeeds() {
    // Ported from Cairo test_assets_below_limit: custom MAX_ASSETS_PER_ACCOUNT=2.
    // With 0 existing assets, a new trade should succeed.
    let mut state = MockStateReader::new();
    let contract = addr(0xA10);
    let trade = make_trade(0x20, 0x21, false, false);

    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(2));

    let mut ctx = crate::ExecutionContext::new();
    let result = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade);
    assert!(result.is_ok(), "should succeed when below custom limit");
}

#[test]
fn test_custom_max_assets_at_limit_fails() {
    // Ported from Cairo test_assets_limit_exceeded: custom MAX_ASSETS_PER_ACCOUNT=1.
    // Maker has 2 existing token assets (exceeds limit of 1). New trade should fail.
    use crate::storage::{storage_key_for_map, storage_key_for_map2};

    let mut state = MockStateReader::new();
    let contract = addr(0xA20);
    let trade = make_trade(0x30, 0x31, false, false);

    // Set custom limit to 1.
    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(1));

    // Give maker 2 token assets via linked list (exceeds limit of 1).
    let maker = trade.maker_order.account;
    let tail_key = storage_key_for_map("Paraclear_token_asset_balance_tail", maker.0);
    set_storage(&mut state, contract, tail_key, felt(0x100));
    // Token 0x100 -> next=0x101
    let base1 = storage_key_for_map2("Paraclear_token_asset_balance", maker.0, felt(0x100));
    set_storage(&mut state, contract, base1, felt(0x100));
    set_storage(&mut state, contract, storage_key_with_offset(base1, 2), felt(0x101));
    // Token 0x101 -> next=0
    let base2 = storage_key_for_map2("Paraclear_token_asset_balance", maker.0, felt(0x101));
    set_storage(&mut state, contract, base2, felt(0x101));

    let mut ctx = crate::ExecutionContext::new();
    let result = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade);
    assert!(result.is_err(), "should fail when maker exceeds custom limit");
    assert!(result.unwrap_err().to_string().contains("maker too many assets"));
}

#[test]
fn test_custom_max_assets_reduce_only_bypasses_limit() {
    // Ported from Cairo test_assets_limit_skipped_on_reduce_only: reduce-only bypasses limit.
    use crate::storage::{storage_key_for_map, storage_key_for_map2};

    let mut state = MockStateReader::new();
    let contract = addr(0xA30);
    let trade = make_trade(0x40, 0x41, true, true); // both reduce_only

    // Set custom limit to 1.
    let base = *paraclear_layout::GLOBAL_CONFIGURATION_BASE;
    set_storage(&mut state, contract, storage_key_with_offset(base, 1), felt(1));

    // Give maker 2 token assets (exceeds limit).
    let maker = trade.maker_order.account;
    let tail_key = storage_key_for_map("Paraclear_token_asset_balance_tail", maker.0);
    set_storage(&mut state, contract, tail_key, felt(0x200));
    let base1 = storage_key_for_map2("Paraclear_token_asset_balance", maker.0, felt(0x200));
    set_storage(&mut state, contract, base1, felt(0x200));
    set_storage(&mut state, contract, storage_key_with_offset(base1, 2), felt(0x201));
    let base2 = storage_key_for_map2("Paraclear_token_asset_balance", maker.0, felt(0x201));
    set_storage(&mut state, contract, base2, felt(0x201));

    let mut ctx = crate::ExecutionContext::new();
    let result = paraclear::enforce_max_assets_per_account_for_test(&mut ctx, &state, contract, &trade);
    assert!(result.is_ok(), "reduce-only should bypass max assets check");
}
