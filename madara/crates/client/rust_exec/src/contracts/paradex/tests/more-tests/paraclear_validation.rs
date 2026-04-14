//! Tests ported from Cairo test_paraclear_trades.cairo: input validation for settle_trade_v3.

use crate::contracts::paradex::paraclear;
use crate::contracts::paradex_codegen::paraclear_types::{OrderCategory, TradeRequestV3};
use crate::storage::function_selector;
use starknet_types_core::felt::Felt;

use super::super::fixtures::{addr, encode_trade_request_v3_for_test, felt, sample_trade};

/// Helper: build a valid base trade with maker(side=1, buy) and taker(side=2, sell).
fn valid_trade() -> TradeRequestV3 {
    let mut trade = sample_trade(OrderCategory::Unspecified, OrderCategory::Unspecified);
    trade.maker_order.side = felt(1); // BUY
    trade.taker_order.side = felt(2); // SELL
    trade.size = felt(10);
    trade.maker_order.size = felt(10);
    trade.taker_order.size = felt(10);
    trade.traded_at = felt(1000);
    trade
}

// ── Size validation ────────────────────────────────────────────────────────

#[test]
fn test_validate_trade_size_zero_rejected() {
    let mut trade = valid_trade();
    trade.size = Felt::ZERO;
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("Size must be >1"), "got: {err}");
}

#[test]
fn test_validate_trade_size_one_rejected() {
    let mut trade = valid_trade();
    trade.size = Felt::ONE;
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("Size must be >1"), "got: {err}");
}

#[test]
fn test_validate_trade_size_two_accepted() {
    let mut trade = valid_trade();
    trade.size = felt(2);
    paraclear::validate_trade_basic_for_test(&trade).expect("size=2 should be valid");
}

// ── Order size validation ──────────────────────────────────────────────────

#[test]
fn test_validate_maker_order_size_less_than_trade_rejected() {
    let mut trade = valid_trade();
    trade.size = felt(10);
    trade.maker_order.size = felt(9); // less than trade size
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("invalid maker order size"), "got: {err}");
}

#[test]
fn test_validate_taker_order_size_less_than_trade_rejected() {
    let mut trade = valid_trade();
    trade.size = felt(10);
    trade.taker_order.size = felt(5); // less than trade size
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("invalid taker order size"), "got: {err}");
}

#[test]
fn test_validate_order_size_equal_to_trade_accepted() {
    let trade = valid_trade(); // sizes all equal
    paraclear::validate_trade_basic_for_test(&trade).expect("equal sizes should be valid");
}

#[test]
fn test_validate_order_size_greater_than_trade_accepted() {
    let mut trade = valid_trade();
    trade.maker_order.size = felt(100); // greater than trade size
    trade.taker_order.size = felt(200);
    paraclear::validate_trade_basic_for_test(&trade).expect("larger order sizes should be valid");
}

// ── Timestamp validation ───────────────────────────────────────────────────

#[test]
fn test_validate_trade_timestamp_zero_rejected() {
    let mut trade = valid_trade();
    trade.traded_at = Felt::ZERO;
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("traded_at must be >0"), "got: {err}");
}

// Note: future timestamp validation is tested separately in paraclear_timing.rs
// (test_execute_with_timestamp_future_trade_fails) because it requires block context.

// ── Market mismatch ────────────────────────────────────────────────────────

#[test]
fn test_validate_trade_market_mismatch_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.market = felt(0x1111);
    trade.taker_order.market = felt(0x2222);
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("Order assets mismatch"), "got: {err}");
}

// ── Self-trade ─────────────────────────────────────────────────────────────

#[test]
fn test_validate_trade_self_trade_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.account = addr(0xAAAA);
    trade.taker_order.account = addr(0xAAAA); // same account
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("self-trades not allowed"), "got: {err}");
}

// ── Same-side orders ───────────────────────────────────────────────────────

#[test]
fn test_validate_trade_same_side_buy_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.side = felt(1); // BUY
    trade.taker_order.side = felt(1); // BUY — same side
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("same side orders"), "got: {err}");
}

#[test]
fn test_validate_trade_same_side_sell_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.side = felt(2); // SELL
    trade.taker_order.side = felt(2); // SELL — same side
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("same side orders"), "got: {err}");
}

// ── Invalid side ───────────────────────────────────────────────────────────

#[test]
fn test_validate_trade_invalid_maker_side_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.side = felt(3); // invalid (only 1 or 2)
    trade.taker_order.side = felt(1);
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("invalid maker order side"), "got: {err}");
}

#[test]
fn test_validate_trade_invalid_taker_side_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.side = felt(1);
    trade.taker_order.side = felt(3); // invalid
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("invalid taker order side"), "got: {err}");
}

#[test]
fn test_validate_trade_invalid_maker_side_zero_rejected() {
    let mut trade = valid_trade();
    trade.maker_order.side = Felt::ZERO; // invalid
    trade.taker_order.side = felt(1);
    let err = paraclear::validate_trade_basic_for_test(&trade).unwrap_err();
    assert!(err.to_string().contains("invalid maker order side"), "got: {err}");
}

// ── Unsupported market (full execution path, needs state) ──────────────────

#[test]
fn test_settle_trade_v3_unsupported_market_returns_zero_with_event() {
    // This test verifies the full execution path: when the market is not supported,
    // settle_trade_v3 returns 0 and emits SettleTradeFailedV3 with error code 1004.
    use super::super::fixtures::{set_paraclear_dependencies, set_settlement_token, short_str};
    use crate::state::mock::MockStateReader;
    use crate::storage::event_selector;

    let mut state = MockStateReader::new();
    let contract = addr(0x5000);
    let assets_manager = addr(0x5001);
    let oracle = addr(0x5002);
    let settlement_token = addr(0x5003);

    set_paraclear_dependencies(&mut state, contract, assets_manager, oracle);
    set_settlement_token(&mut state, contract, settlement_token, short_str("USDC"));
    // Oracle price for settlement token.
    super::super::fixtures::set_oracle_latest_tick_data(
        &mut state,
        oracle,
        short_str("USDC"),
        short_str("USDC"),
        felt(100_000_000),
        felt(8),
    );

    let mut trade = valid_trade();
    trade.maker_order.market = felt(0xDEAD); // unsupported market
    trade.taker_order.market = felt(0xDEAD);

    let calldata = encode_trade_request_v3_for_test(&trade);
    let selector = function_selector("settle_trade_v3");
    let result = paraclear::execute(&state, contract, selector, &calldata, addr(0x999)).expect("execute");

    // Should return 0 (failure).
    assert_eq!(result.call_result.retdata, vec![Felt::ZERO]);

    // Should emit SettleTradeFailedV3 event.
    let failed_events: Vec<_> = result
        .call_result
        .events
        .iter()
        .filter(|e| e.keys.first() == Some(&event_selector("SettleTradeFailedV3")))
        .collect();
    assert_eq!(failed_events.len(), 1, "expected 1 SettleTradeFailedV3 event");

    // Error code 1004 = TRADE_MARKET_NOT_SUPPORTED.
    assert_eq!(failed_events[0].data[0], felt(1004));
}
