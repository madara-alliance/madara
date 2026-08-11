//! Centralized constants for rust-exec integration points.
//!
//! We intentionally keep these values in one place so:
//! - traceTransactionRust and block production use the same numbers.
//! - changing fixed receipt/resources/bouncer weights is a single edit.

use crate::core::gas::GasVector;

// NOTE: These numbers are currently hardcoded based on an empirical Blockifier run of
// `settle_trade_v3`. They are a temporary bridge until rust-exec produces receipts/resources
// natively with full fidelity.

pub const SETTLE_TRADE_V3_FIXED_FEE_AMOUNT: u128 = 0x0263_9288_be94;
pub const SETTLE_TRADE_V3_FIXED_L1_GAS: u64 = 0;
pub const SETTLE_TRADE_V3_FIXED_L1_DATA_GAS: u64 = 576;
pub const SETTLE_TRADE_V3_FIXED_L2_GAS: u64 = 25_696_375;

pub fn settle_trade_v3_fixed_fee_amount() -> u128 {
    SETTLE_TRADE_V3_FIXED_FEE_AMOUNT
}

pub fn settle_trade_v3_fixed_gas_consumed() -> GasVector {
    GasVector {
        l1_gas: SETTLE_TRADE_V3_FIXED_L1_GAS,
        l1_data_gas: SETTLE_TRADE_V3_FIXED_L1_DATA_GAS,
        l2_gas: SETTLE_TRADE_V3_FIXED_L2_GAS,
    }
}

// Bouncer weights are used to decide block closure. For rust-exec we derive the dominant terms
// (sierra/proving gas) from empirical Blockifier runs, indexed by positions count.
//
// Configure `settle_trade_v3_positions` in the Madara Rust Exec config to pick the nearest available
// bucket (ties go to the higher bucket). If unset, we fall back to defaults.
pub const SETTLE_TRADE_V3_BOUNCER_DEFAULT_FIRST_SIERRA_GAS: u64 = 600_124_824;
pub const SETTLE_TRADE_V3_BOUNCER_DEFAULT_FIRST_PROVING_GAS: u64 = 606_943_554;
pub const SETTLE_TRADE_V3_BOUNCER_DEFAULT_STEADY_SIERRA_GAS: u64 = 37_282_620;
pub const SETTLE_TRADE_V3_BOUNCER_DEFAULT_STEADY_PROVING_GAS: u64 = 41_409_414;

#[derive(Clone, Copy)]
struct BouncerGasProfile {
    positions: u16,
    first_sierra: u64,
    first_proving: u64,
    steady_sierra: u64,
    steady_proving: u64,
}

const BOUNCER_GAS_PROFILES: &[BouncerGasProfile] = &[
    BouncerGasProfile {
        positions: 0,
        first_sierra: 585_460_806,
        first_proving: 589_711_592,
        steady_sierra: 13_221_632,
        steady_proving: 16_160_962,
    },
    BouncerGasProfile {
        positions: 1,
        first_sierra: 584_774_454,
        first_proving: 588_965_958,
        steady_sierra: 12_853_120,
        steady_proving: 15_767_680,
    },
    BouncerGasProfile {
        positions: 5,
        first_sierra: 600_970_614,
        first_proving: 606_375_790,
        steady_sierra: 23_082_880,
        steady_proving: 26_451_848,
    },
    BouncerGasProfile {
        positions: 10,
        first_sierra: 621_215_814,
        first_proving: 628_138_080,
        steady_sierra: 35_870_080,
        steady_proving: 39_807_058,
    },
    BouncerGasProfile {
        positions: 50,
        first_sierra: 783_177_414,
        first_proving: 802_236_400,
        steady_sierra: 138_167_680,
        steady_proving: 146_648_738,
    },
    BouncerGasProfile {
        positions: 100,
        first_sierra: 985_629_414,
        first_proving: 1_019_859_300,
        steady_sierra: 266_039_680,
        steady_proving: 280_200_838,
    },
    BouncerGasProfile {
        positions: 140,
        first_sierra: 1_147_591_014,
        first_proving: 1_193_957_620,
        steady_sierra: 368_337_280,
        steady_proving: 387_042_518,
    },
];

fn select_bouncer_profile(positions: u16) -> BouncerGasProfile {
    let mut best = BOUNCER_GAS_PROFILES[0];
    let mut best_diff = positions.abs_diff(best.positions);
    for profile in BOUNCER_GAS_PROFILES.iter().copied().skip(1) {
        let diff = positions.abs_diff(profile.positions);
        if diff < best_diff || (diff == best_diff && profile.positions > best.positions) {
            best = profile;
            best_diff = diff;
        }
    }
    best
}

pub fn settle_trade_v3_bouncer_gas(is_first: bool) -> (u64, u64) {
    if let Some(profile) = crate::config::settle_trade_v3_positions().map(select_bouncer_profile) {
        if is_first {
            (profile.first_sierra, profile.first_proving)
        } else {
            (profile.steady_sierra, profile.steady_proving)
        }
    } else if is_first {
        (SETTLE_TRADE_V3_BOUNCER_DEFAULT_FIRST_SIERRA_GAS, SETTLE_TRADE_V3_BOUNCER_DEFAULT_FIRST_PROVING_GAS)
    } else {
        (SETTLE_TRADE_V3_BOUNCER_DEFAULT_STEADY_SIERRA_GAS, SETTLE_TRADE_V3_BOUNCER_DEFAULT_STEADY_PROVING_GAS)
    }
}
