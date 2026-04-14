//! Layer 1: Encode/decode roundtrip tests for trade-related data structures.
//!
//! These tests verify that encoding a struct to calldata and decoding it back
//! produces the original struct. No blockifier needed.

mod fuzz_common;

use mc_rust_exec::contracts::paradex::paraclear;
use mc_rust_exec::contracts::paradex_codegen::paraclear_types::{OrderCategory, OrderV3, TradeRequestV3};
use mc_rust_exec::types::ContractAddress;
use proptest::prelude::*;
use starknet_types_core::felt::Felt;

use fuzz_common::strategies::{arb_contract_address, arb_order_category, arb_valid_trade, fuzz_cases};

// ── OrderCategory roundtrip ─────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(5_000)))]

    #[test]
    fn order_category_roundtrip(cat in arb_order_category()) {
        let encoded = paraclear::encode_order_category_for_test(&cat);
        let (decoded, rest) = paraclear::decode_order_category_for_test(&encoded).unwrap();
        prop_assert!(rest.is_empty(), "leftover bytes after decoding OrderCategory");
        prop_assert_eq!(decoded, cat);
    }
}

// ── OrderV3 roundtrip ───────────────────────────────────────────────────────

fn arb_standalone_order() -> impl Strategy<Value = OrderV3> {
    (
        arb_contract_address(),
        (1u64..=u64::MAX).prop_map(Felt::from),               // market
        prop_oneof![Just(Felt::ONE), Just(Felt::from(2u64))], // side
        (0u64..=10u64).prop_map(Felt::from),                  // orderType
        (1u64..=1_000_000u64).prop_map(Felt::from),           // size
        (1u64..=1_000_000_000u64).prop_map(Felt::from),       // price
        (1u64..=u32::MAX as u64).prop_map(Felt::from),        // sig timestamp
        any::<bool>(),                                        // is_reduce_only
        arb_order_category(),                                 // order_category
    )
        .prop_map(|(account, market, side, order_type, size, price, ts, reduce_only, cat)| OrderV3 {
            account,
            market,
            side,
            orderType: order_type,
            size,
            price,
            signature_timestamp: ts,
            is_reduce_only: reduce_only,
            order_category: cat,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(5_000)))]

    #[test]
    fn order_v3_roundtrip(order in arb_standalone_order()) {
        let encoded = paraclear::encode_order_v3_for_test(&order);
        let (decoded, rest) = paraclear::decode_order_v3_for_test(&encoded).unwrap();
        prop_assert!(rest.is_empty(), "leftover bytes after decoding OrderV3");
        prop_assert_eq!(decoded, order);
    }
}

// ── TradeRequestV3 roundtrip ────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(5_000)))]

    #[test]
    fn trade_request_v3_roundtrip(trade in arb_valid_trade()) {
        let encoded = paraclear::encode_trade_request_v3_for_test(&trade);
        let (decoded, rest) = paraclear::decode_trade_request_v3(&encoded).unwrap();
        prop_assert!(rest.is_empty(), "leftover bytes after decoding TradeRequestV3");
        prop_assert_eq!(decoded, trade);
    }
}

// ── Edge cases: truncated calldata ──────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(1_000)))]

    /// Truncating a valid TradeRequestV3 encoding at any position should produce an error,
    /// never a panic.
    #[test]
    fn trade_request_v3_truncated_no_panic(
        trade in arb_valid_trade(),
        truncate_at in 0usize..30usize,
    ) {
        let encoded = paraclear::encode_trade_request_v3_for_test(&trade);
        let truncate_pos = truncate_at.min(encoded.len().saturating_sub(1));
        if truncate_pos < encoded.len() {
            let truncated: Vec<_> = encoded[..truncate_pos].to_vec();
            // Should return Err, never panic
            let result = paraclear::decode_trade_request_v3(&truncated);
            prop_assert!(result.is_err(), "truncated at {} should error, got Ok", truncate_pos);
        }
    }

    /// Single-element calldata should fail decode gracefully.
    #[test]
    fn trade_request_v3_minimal_calldata(value in (0u64..100u64).prop_map(Felt::from)) {
        let data = vec![value];
        let result = paraclear::decode_trade_request_v3(&data);
        prop_assert!(result.is_err());
    }
}

// ── validate_trade_basic property tests ─────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(5_000)))]

    /// A structurally valid trade (from arb_valid_trade) should pass basic validation.
    #[test]
    fn valid_trade_passes_basic_validation(trade in arb_valid_trade()) {
        let result = paraclear::validate_trade_basic_for_test(&trade);
        prop_assert!(
            result.is_ok(),
            "valid trade failed basic validation: {:?}",
            result.err()
        );
    }

    /// Self-trade (same account for maker and taker) should be rejected.
    #[test]
    fn self_trade_rejected(
        account in arb_contract_address(),
        market in (1u64..=u64::MAX).prop_map(Felt::from),
        size in (2u64..=10_000u64).prop_map(Felt::from),
        price in (1u64..=1_000_000u64).prop_map(Felt::from),
    ) {
        let trade = TradeRequestV3 {
            id: Felt::from(1u64),
            size,
            price,
            traded_at: Felt::from(1000u64),
            maker_order: OrderV3 {
                account,
                market,
                side: Felt::ONE,
                orderType: Felt::ZERO,
                size,
                price,
                signature_timestamp: Felt::from(1u64),
                is_reduce_only: false,
                order_category: OrderCategory::Unspecified,
            },
            taker_order: OrderV3 {
                account, // Same account!
                market,
                side: Felt::from(2u64),
                orderType: Felt::ZERO,
                size,
                price,
                signature_timestamp: Felt::from(1u64),
                is_reduce_only: false,
                order_category: OrderCategory::Unspecified,
            },
        };
        let result = paraclear::validate_trade_basic_for_test(&trade);
        prop_assert!(result.is_err(), "self-trade should be rejected");
    }

    /// Same-side orders (both buy or both sell) should be rejected.
    #[test]
    fn same_side_rejected(
        side in prop_oneof![Just(Felt::ONE), Just(Felt::from(2u64))],
        size in (2u64..=10_000u64).prop_map(Felt::from),
    ) {
        let trade = TradeRequestV3 {
            id: Felt::from(1u64),
            size,
            price: Felt::from(100u64),
            traded_at: Felt::from(1000u64),
            maker_order: OrderV3 {
                account: ContractAddress(Felt::from(1u64)),
                market: Felt::from(0x2222u64),
                side, // Same side!
                orderType: Felt::ZERO,
                size,
                price: Felt::from(100u64),
                signature_timestamp: Felt::from(1u64),
                is_reduce_only: false,
                order_category: OrderCategory::Unspecified,
            },
            taker_order: OrderV3 {
                account: ContractAddress(Felt::from(2u64)),
                market: Felt::from(0x2222u64),
                side, // Same side!
                orderType: Felt::ZERO,
                size,
                price: Felt::from(100u64),
                signature_timestamp: Felt::from(1u64),
                is_reduce_only: false,
                order_category: OrderCategory::Unspecified,
            },
        };
        let result = paraclear::validate_trade_basic_for_test(&trade);
        prop_assert!(result.is_err(), "same-side orders should be rejected");
    }
}
