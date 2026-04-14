//! Layer 0: Property-based tests for pure math functions in paraclear.
//!
//! These tests verify algebraic properties of the fixed-point arithmetic functions
//! without needing any state or blockifier. They run at ~100k iterations/sec.

mod fuzz_common;

use mc_rust_exec::contracts::paradex::paraclear;
use proptest::prelude::*;

use fuzz_common::strategies::fuzz_cases;

const MULTIPLIER: i128 = paraclear::MULTIPLIER_FOR_TEST;

// ── felt <-> i128 roundtrip ─────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(10_000)))]

    /// For all i64 values (subset of i128), encoding to Felt and back must be lossless.
    #[test]
    fn felt_i128_roundtrip_i64(value in i64::MIN..=i64::MAX) {
        let value = value as i128;
        let felt = paraclear::i128_to_felt_for_test(value);
        let back = paraclear::felt_to_i128_for_test(felt).unwrap();
        prop_assert_eq!(back, value, "roundtrip failed for {}", value);
    }

    /// Roundtrip over the full positive i128 range.
    #[test]
    fn felt_i128_roundtrip_positive(value in 0i128..=i128::MAX) {
        let felt = paraclear::i128_to_felt_for_test(value);
        let back = paraclear::felt_to_i128_for_test(felt).unwrap();
        prop_assert_eq!(back, value);
    }

    /// Roundtrip over the full negative i128 range.
    #[test]
    fn felt_i128_roundtrip_negative(value in i128::MIN..=-1i128) {
        let felt = paraclear::i128_to_felt_for_test(value);
        let back = paraclear::felt_to_i128_for_test(felt).unwrap();
        prop_assert_eq!(back, value);
    }
}

// ── mul_128_scaled properties ───────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(10_000)))]

    /// Commutativity: mul(a, b) == mul(b, a)
    #[test]
    fn mul_128_scaled_commutative(
        a in -1_000_000_000i128..=1_000_000_000i128,
        b in -1_000_000_000i128..=1_000_000_000i128,
    ) {
        let ab = paraclear::mul_128_scaled_for_test(a, b);
        let ba = paraclear::mul_128_scaled_for_test(b, a);
        match (ab, ba) {
            (Ok(ab_val), Ok(ba_val)) => prop_assert_eq!(ab_val, ba_val),
            (Err(_), Err(_)) => {} // Both overflow — acceptable
            _ => prop_assert!(false, "one overflowed but not the other"),
        }
    }

    /// Identity: mul(a, MULTIPLIER) == a
    #[test]
    fn mul_128_scaled_identity(a in -1_000_000_000_000i128..=1_000_000_000_000i128) {
        let result = paraclear::mul_128_scaled_for_test(a, MULTIPLIER).unwrap();
        prop_assert_eq!(result, a);
    }

    /// Zero: mul(a, 0) == 0 and mul(0, b) == 0
    #[test]
    fn mul_128_scaled_zero(a in i64::MIN as i128..=i64::MAX as i128) {
        let result_a0 = paraclear::mul_128_scaled_for_test(a, 0).unwrap();
        let result_0a = paraclear::mul_128_scaled_for_test(0, a).unwrap();
        prop_assert_eq!(result_a0, 0);
        prop_assert_eq!(result_0a, 0);
    }

    /// Sign: mul(positive, positive) >= 0, mul(positive, negative) <= 0
    #[test]
    fn mul_128_scaled_sign(
        a in 1i128..=1_000_000_000i128,
        b in 1i128..=1_000_000_000i128,
    ) {
        let pos_pos = paraclear::mul_128_scaled_for_test(a, b).unwrap();
        let pos_neg = paraclear::mul_128_scaled_for_test(a, -b).unwrap();
        let neg_neg = paraclear::mul_128_scaled_for_test(-a, -b).unwrap();

        prop_assert!(pos_pos >= 0, "pos*pos should be non-negative");
        prop_assert!(pos_neg <= 0, "pos*neg should be non-positive");
        prop_assert!(neg_neg >= 0, "neg*neg should be non-negative");
    }
}

// ── div_128_scaled properties ───────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(10_000)))]

    /// Identity: div(a, MULTIPLIER) == a
    #[test]
    fn div_128_scaled_identity(a in -1_000_000_000i128..=1_000_000_000i128) {
        let result = paraclear::div_128_scaled_for_test(a, MULTIPLIER).unwrap();
        prop_assert_eq!(result, a);
    }

    /// Zero numerator: div(0, b) == 0
    #[test]
    fn div_128_scaled_zero_numerator(b in 1i128..=1_000_000_000i128) {
        let result = paraclear::div_128_scaled_for_test(0, b).unwrap();
        prop_assert_eq!(result, 0);
    }

    /// Division by zero returns error
    #[test]
    fn div_128_scaled_division_by_zero(a in i64::MIN as i128..=i64::MAX as i128) {
        let result = paraclear::div_128_scaled_for_test(a, 0);
        prop_assert!(result.is_err());
    }

    /// mul/div inverse: div(mul(a, b), b) ≈ a (within rounding)
    ///
    /// Fixed-point arithmetic loses precision when the intermediate product is small.
    /// For example: mul(119, 847458) = 1, then div(1, 847458) = 117, not 119.
    /// We only check the inverse when the product is large enough to preserve precision.
    #[test]
    fn mul_div_inverse(
        a in 1i128..=1_000_000i128,
        b in 1i128..=1_000_000i128,
    ) {
        let product = paraclear::mul_128_scaled_for_test(a, b);
        if let Ok(product) = product {
            // Only check when product is large enough that rounding is small relative to a
            if product >= MULTIPLIER {
                let back = paraclear::div_128_scaled_for_test(product, b).unwrap();
                // Allow ±1 rounding error
                prop_assert!(
                    (back - a).abs() <= 1,
                    "mul/div inverse failed: a={}, b={}, product={}, back={}",
                    a, b, product, back
                );
            }
        }
    }
}

// ── mul3_128_scaled properties ──────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(5_000)))]

    /// Zero: mul3(a, b, 0) == 0
    #[test]
    fn mul3_128_scaled_zero(
        a in -1_000_000i128..=1_000_000i128,
        b in -1_000_000i128..=1_000_000i128,
    ) {
        let result = paraclear::mul3_128_scaled_for_test(a, b, 0).unwrap();
        prop_assert_eq!(result, 0);
    }

    /// mul3(a, MULTIPLIER, MULTIPLIER) == a
    #[test]
    fn mul3_128_scaled_double_identity(a in -1_000_000i128..=1_000_000i128) {
        let result = paraclear::mul3_128_scaled_for_test(a, MULTIPLIER, MULTIPLIER).unwrap();
        prop_assert_eq!(result, a);
    }

    /// Approximate associativity: mul3(a,b,c) ≈ mul(mul(a,b), c) within rounding
    #[test]
    fn mul3_128_scaled_vs_chained(
        a in 1i128..=10_000i128,
        b in 1i128..=10_000i128,
        c in 1i128..=10_000i128,
    ) {
        let direct = paraclear::mul3_128_scaled_for_test(a, b, c);
        let chained = paraclear::mul_128_scaled_for_test(a, b)
            .and_then(|ab| paraclear::mul_128_scaled_for_test(ab, c));

        if let (Ok(d), Ok(ch)) = (direct, chained) {
            // Allow ±1 rounding error from different division order
            prop_assert!(
                (d - ch).abs() <= 1,
                "mul3 vs chained: a={}, b={}, c={}, direct={}, chained={}",
                a, b, c, d, ch
            );
        }
    }
}

// ── abs_128 ─────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(10_000)))]

    /// abs is always non-negative (except i128::MIN which wraps)
    #[test]
    fn abs_128_non_negative(value in (i128::MIN + 1)..=i128::MAX) {
        let result = paraclear::abs_128_for_test(value);
        prop_assert!(result >= 0, "abs({}) = {} should be non-negative", value, result);
    }

    /// abs(x) == abs(-x) for x > 0
    #[test]
    fn abs_128_symmetric(value in 1i128..=i128::MAX) {
        let pos = paraclear::abs_128_for_test(value);
        let neg = paraclear::abs_128_for_test(-value);
        prop_assert_eq!(pos, neg);
    }

    /// abs(0) == 0
    #[test]
    fn abs_128_zero(_dummy in 0u8..1u8) {
        let result = paraclear::abs_128_for_test(0);
        prop_assert_eq!(result, 0);
    }
}

// ── Overflow detection ──────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(fuzz_cases(5_000)))]

    /// Large values should return Err, never panic.
    #[test]
    fn mul_128_scaled_no_panic_on_large_values(
        a in i64::MIN as i128..=i64::MAX as i128,
        b in i64::MIN as i128..=i64::MAX as i128,
    ) {
        // This should never panic, only Ok or Err
        let _ = paraclear::mul_128_scaled_for_test(a, b);
    }

    /// div with large numerator should return Err or valid result, never panic.
    #[test]
    fn div_128_scaled_no_panic_on_large_values(
        a in i64::MIN as i128..=i64::MAX as i128,
        b in 1i128..=i64::MAX as i128,
    ) {
        let _ = paraclear::div_128_scaled_for_test(a, b);
    }
}
