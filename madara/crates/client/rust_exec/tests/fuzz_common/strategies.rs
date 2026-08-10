//! Proptest strategies for generating domain types used in differential fuzzing.
#![allow(dead_code)]

use mc_rust_exec::contracts::paradex::paraclear;
use mc_rust_exec::contracts::paradex::schema::paraclear_types::{
    FeeWithCapRequest, OrderCategory, OrderV3, TradeRequestV3,
};
use mc_rust_exec::types::ContractAddress;
use proptest::prelude::*;
use starknet_types_core::felt::Felt;

// ── Primitive strategies ────────────────────────────────────────────────────

/// Non-zero contract address that fits within the StarkFelt valid range.
pub fn arb_contract_address() -> impl Strategy<Value = ContractAddress> {
    (1u64..=0xFFFF_FFFFu64).prop_map(|v| ContractAddress(Felt::from(v)))
}

/// Felt representing an i128 value in the given range, encoded via i128_to_felt.
pub fn arb_felt_i128_range(lo: i128, hi: i128) -> impl Strategy<Value = Felt> {
    (lo..=hi).prop_map(paraclear::i128_to_felt_for_test)
}

/// Felt representing a positive i128 value suitable for prices/sizes.
pub fn arb_felt_positive(max: i128) -> impl Strategy<Value = Felt> {
    (1i128..=max).prop_map(|v| Felt::from(v as u128))
}

/// Non-zero Felt from u64 range.
pub fn arb_felt_nonzero() -> impl Strategy<Value = Felt> {
    (1u64..=u64::MAX).prop_map(Felt::from)
}

// ── Domain type strategies ──────────────────────────────────────────────────

pub fn arb_fee_with_cap_request() -> impl Strategy<Value = FeeWithCapRequest> {
    // fee_floor <= fee <= fee_cap for consistent behavior
    (0u64..=500_000, 0u64..=500_000, 0u64..=500_000).prop_map(|(fee, cap_delta, floor)| FeeWithCapRequest {
        fee: Felt::from(fee),
        fee_cap: Felt::from(fee.saturating_add(cap_delta)),
        fee_floor: Felt::from(floor.min(fee)),
    })
}

pub fn arb_order_category() -> impl Strategy<Value = OrderCategory> {
    prop_oneof![
        Just(OrderCategory::Unspecified),
        Just(OrderCategory::API),
        Just(OrderCategory::RPI),
        Just(OrderCategory::Interactive),
        arb_fee_with_cap_request().prop_map(OrderCategory::Dynamic),
    ]
}

/// Generate an OrderV3 with the given fixed account, market, side, and minimum size.
pub fn arb_order_v3(
    account: ContractAddress,
    market: Felt,
    side: Felt,
    min_size: i128,
) -> impl Strategy<Value = OrderV3> {
    (
        (min_size..=min_size.saturating_mul(10)), // size >= trade size
        (1i128..=1_000_000_000i128),              // price (raw, not scaled)
        (1u64..=u32::MAX as u64),                 // signature_timestamp
        any::<bool>(),                            // is_reduce_only
        arb_order_category(),                     // order_category
    )
        .prop_map(move |(size, price, ts, reduce_only, cat)| OrderV3 {
            account,
            market,
            side,
            orderType: Felt::ZERO,
            size: Felt::from(size as u128),
            price: Felt::from(price as u128),
            signature_timestamp: Felt::from(ts),
            is_reduce_only: reduce_only,
            order_category: cat,
        })
}

/// Generate a structurally valid TradeRequestV3.
///
/// Enforces:
/// - trade size > 1
/// - maker and taker on opposite sides
/// - maker account != taker account
/// - same market for both orders
/// - order sizes >= trade size
pub fn arb_valid_trade() -> impl Strategy<Value = TradeRequestV3> {
    (
        arb_contract_address(),      // maker
        arb_contract_address(),      // taker
        arb_felt_nonzero(),          // market
        (2i128..=10_000i128),        // trade size (>1)
        (1i128..=1_000_000_000i128), // trade price
        (1u64..=1_000_000u64),       // traded_at (>0)
        any::<bool>(),               // maker_is_buyer
        arb_order_category(),        // maker category
        arb_order_category(),        // taker category
    )
        .prop_filter("maker != taker", |(m, t, _, _, _, _, _, _, _)| m != t)
        .prop_flat_map(|(maker, taker, market, size, price, traded_at, maker_is_buyer, mcat, tcat)| {
            let (maker_side, taker_side) =
                if maker_is_buyer { (Felt::ONE, Felt::from(2u64)) } else { (Felt::from(2u64), Felt::ONE) };
            (
                arb_order_v3(maker, market, maker_side, size).prop_map(move |mut o| {
                    o.order_category = mcat.clone();
                    o
                }),
                arb_order_v3(taker, market, taker_side, size).prop_map(move |mut o| {
                    o.order_category = tcat.clone();
                    o
                }),
            )
                .prop_map(move |(maker_order, taker_order)| TradeRequestV3 {
                    id: Felt::from(1u64),
                    size: Felt::from(size as u128),
                    price: Felt::from(price as u128),
                    traded_at: Felt::from(traded_at),
                    maker_order,
                    taker_order,
                })
        })
}

// ── Utilities ───────────────────────────────────────────────────────────────

/// Read FUZZ_CASES env var, defaulting to `default_val`.
pub fn fuzz_cases(default_val: u32) -> u32 {
    std::env::var("FUZZ_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_val)
}
