//! Precomputed sn_keccak values for Paradex-related string constants.
//!
//! In blockifier + Cairo Native, many selectors and storage base keys are compiled into
//! constants and do not require runtime hashing. Rust-exec computes these at runtime to
//! match the exact Starknet storage layout and selector rules, so we maintain a map here
//! to skip repeat hashing for known string inputs.
//!
//! Flow:
//! - sn_keccak -> lookup map
//! - miss -> log (so we can add it later) and fall back to real hashing
//! - hit -> return precomputed Felt

use once_cell::sync::Lazy;
use sha3::{Digest, Keccak256};
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

static PRECOMPUTED_SN_KECCAK_ENABLED: Lazy<bool> = Lazy::new(|| {
    let value = std::env::var("RUST_EXEC_PRECOMPUTED_SN_KECCAK").unwrap_or_default();
    if value.is_empty() {
        return false;
    }
    match value.to_ascii_lowercase().as_str() {
        "0" | "false" | "no" | "off" => false,
        _ => true,
    }
});

const PRECOMPUTED_VALUES: &[&str] = &[
    "ERC20_balances",
    "Fee",
    "Paraclear_account_fee_rate",
    "Paraclear_account_referral",
    "Paraclear_fee_account_address",
    "Paraclear_fee_share_account_address",
    "Paraclear_fee_share_percentage",
    "Paraclear_global_fee_rate",
    "Paraclear_market_delegate",
    "Paraclear_oracle_contract_address",
    "Paraclear_perpetual_asset",
    "Paraclear_perpetual_asset_balance",
    "Paraclear_perpetual_asset_balance_tail",
    "Paraclear_settlement_token_asset",
    "Paraclear_token_asset",
    "Paraclear_token_asset_balance",
    "Paraclear_token_asset_balance_tail",
    "Paraclear_transfer_registry",
    "PerpetualAssetBalanceUpdateV3",
    "TokenAssetBalanceUpdate",
    "TradeSettled",
    "Transfer",
    "__execute__",
    "__validate__",
    "__validate_deploy__",
    "account_margin_methodology",
    "assets_manager",
    "decrement_pending_trade",
    "funding_index_data",
    "get_value",
    "get_values_with_funding_indices",
    "global_configuration",
    "invariants_perpetual_asset_info",
    "latest_tick_data",
    "latest_updated_timestamp",
    "perpetual_future_market_fee_config_v2",
    "perpetual_futures_mmf_factor",
    "settle_trade_v3",
    "transfer",
];

static PRECOMPUTED_SN_KECCAK: Lazy<HashMap<&'static str, Felt>> = Lazy::new(|| {
    let mut map = HashMap::with_capacity(PRECOMPUTED_VALUES.len());
    for value in PRECOMPUTED_VALUES {
        map.insert(*value, compute_sn_keccak_bytes(value.as_bytes()));
    }
    map
});

fn compute_sn_keccak_bytes(data: &[u8]) -> Felt {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let mut result: [u8; 32] = hasher.finalize().into();
    result[0] &= 0x03; // Starknet keccak: mask to 250 bits.
    Felt::from_bytes_be(&result)
}

pub fn lookup_sn_keccak(data: &[u8]) -> Option<Felt> {
    if !*PRECOMPUTED_SN_KECCAK_ENABLED {
        return None;
    }
    let Ok(value) = std::str::from_utf8(data) else {
        return None;
    };
    if let Some(felt) = PRECOMPUTED_SN_KECCAK.get(value) {
        return Some(*felt);
    }
    None
}
