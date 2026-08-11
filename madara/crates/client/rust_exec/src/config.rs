//! Configuration for Rust native execution.
//!
//! The current pipeline initializes RustExec explicitly from Madara block production.

use once_cell::sync::OnceCell;
use starknet_types_core::felt::Felt;
use std::collections::HashSet;

/// Parse a hex string into a Felt.
///
/// Accepts formats:
/// - `0x123abc...` (with prefix)
/// - `123abc...` (without prefix)
#[cfg(test)]
fn parse_felt(s: &str) -> Result<Felt, String> {
    let hex_str = s.strip_prefix("0x").unwrap_or(s);

    if hex_str.is_empty() {
        return Err("Empty hex string".to_string());
    }

    // Validate hex characters
    if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid hex characters".to_string());
    }

    Felt::from_hex(s).map_err(|e| format!("Failed to parse Felt: {:?}", e))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustExecRuntimeConfig {
    pub account_class_hash: Felt,
    pub erc20_class_hash: Felt,
    pub paraclear_class_hash: Felt,
    pub paraclear_oracle_class_hash: Felt,
    pub assets_manager_class_hash: Felt,
    pub supported_contract_class_hashes: HashSet<Felt>,
    pub settle_trade_fixed_fee: u128,
    pub no_charge_fee: bool,
    pub conversion_log: bool,
    pub execution_log: bool,
    pub execution_log_inner: bool,
    pub tx_diff_log: bool,
    pub debug_block: Option<u64>,
    pub inner_timing_log: bool,
    pub ctx_cache: bool,
    pub pedersen_cache: bool,
    pub precomputed_sn_keccak: bool,
    pub hash_agg_logs: bool,
    pub storage_agg_logs: bool,
    pub ignore_fee_mismatch: bool,
    pub settle_trade_v3_positions: Option<u16>,
}

impl Default for RustExecRuntimeConfig {
    fn default() -> Self {
        Self {
            // Reused from paradex_optimisations CI defaults.
            account_class_hash: Felt::from_hex_unchecked(
                "0xe81f6009f96661c969f14c40d8b453cc40fc6c674607a61c23bb3563709e2a",
            ),
            erc20_class_hash: Felt::from_hex_unchecked(
                "0x072f69eca0f2c114a0125e85120697b37a3d71ce116ff54a6af84d956c37bfa4",
            ),
            paraclear_class_hash: Felt::ZERO,
            paraclear_oracle_class_hash: Felt::ZERO,
            assets_manager_class_hash: Felt::ZERO,
            supported_contract_class_hashes: HashSet::new(),
            settle_trade_fixed_fee: crate::constants::SETTLE_TRADE_V3_FIXED_FEE_AMOUNT,
            no_charge_fee: false,
            conversion_log: false,
            execution_log: false,
            execution_log_inner: false,
            tx_diff_log: false,
            debug_block: None,
            inner_timing_log: false,
            ctx_cache: true,
            pedersen_cache: true,
            precomputed_sn_keccak: false,
            hash_agg_logs: false,
            storage_agg_logs: false,
            ignore_fee_mismatch: false,
            settle_trade_v3_positions: None,
        }
    }
}

static RUNTIME_CONFIG: OnceCell<RustExecRuntimeConfig> = OnceCell::new();

fn nonzero(value: Felt) -> Option<Felt> {
    (value != Felt::ZERO).then_some(value)
}

pub fn account_class_hash() -> Option<Felt> {
    runtime_config().and_then(|cfg| nonzero(cfg.account_class_hash))
}

pub fn erc20_class_hash() -> Option<Felt> {
    runtime_config().and_then(|cfg| nonzero(cfg.erc20_class_hash))
}

pub fn paraclear_class_hash() -> Option<Felt> {
    runtime_config().and_then(|cfg| nonzero(cfg.paraclear_class_hash))
}

pub fn paraclear_oracle_class_hash() -> Option<Felt> {
    runtime_config().and_then(|cfg| nonzero(cfg.paraclear_oracle_class_hash))
}

pub fn assets_manager_class_hash() -> Option<Felt> {
    runtime_config().and_then(|cfg| nonzero(cfg.assets_manager_class_hash))
}

pub fn supports_runtime_class_hash(class_hash: Felt) -> bool {
    runtime_config().map(|cfg| cfg.supported_contract_class_hashes.contains(&class_hash)).unwrap_or(false)
}

pub fn runtime_config() -> Option<&'static RustExecRuntimeConfig> {
    RUNTIME_CONFIG.get()
}

pub fn initialize_runtime_config(config: RustExecRuntimeConfig) {
    let _ = RUNTIME_CONFIG.set(config);
}

fn runtime_bool(get: impl FnOnce(&RustExecRuntimeConfig) -> bool, default: bool) -> bool {
    runtime_config().map(get).unwrap_or(default)
}

pub fn no_charge_fee_enabled() -> bool {
    runtime_bool(|cfg| cfg.no_charge_fee, false)
}

pub fn conversion_log_enabled() -> bool {
    runtime_bool(|cfg| cfg.conversion_log, false)
}

pub fn execution_log_enabled() -> bool {
    runtime_bool(|cfg| cfg.execution_log, false)
}

pub fn execution_log_inner_enabled() -> bool {
    runtime_bool(|cfg| cfg.execution_log_inner, false)
}

pub fn tx_diff_log_enabled() -> bool {
    runtime_bool(|cfg| cfg.tx_diff_log, false)
}

pub fn debug_block() -> Option<u64> {
    runtime_config().and_then(|cfg| cfg.debug_block)
}

pub fn inner_timing_log_enabled() -> bool {
    runtime_bool(|cfg| cfg.inner_timing_log, false)
}

pub fn ctx_cache_enabled() -> bool {
    runtime_bool(|cfg| cfg.ctx_cache, true)
}

pub fn pedersen_cache_enabled() -> bool {
    runtime_bool(|cfg| cfg.pedersen_cache, true)
}

pub fn precomputed_sn_keccak_enabled() -> bool {
    runtime_bool(|cfg| cfg.precomputed_sn_keccak, false)
}

pub fn hash_agg_logs_enabled() -> bool {
    runtime_bool(|cfg| cfg.hash_agg_logs, false)
}

pub fn storage_agg_logs_enabled() -> bool {
    runtime_bool(|cfg| cfg.storage_agg_logs, false)
}

pub fn ignore_fee_mismatch() -> bool {
    runtime_bool(|cfg| cfg.ignore_fee_mismatch, false)
}

pub fn settle_trade_v3_positions() -> Option<u16> {
    runtime_config().and_then(|cfg| cfg.settle_trade_v3_positions)
}

/// Check if Rust verification is enabled for any contracts.
///
/// Returns `true` if at least one contract class hash is configured.
pub fn is_verification_enabled() -> bool {
    account_class_hash().is_some()
        || erc20_class_hash().is_some()
        || paraclear_class_hash().is_some()
        || paraclear_oracle_class_hash().is_some()
        || assets_manager_class_hash().is_some()
}

/// Log the current configuration status.
pub fn log_config_status() {
    let _ = is_verification_enabled();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_felt_with_prefix() {
        let result = parse_felt("0x123abc");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_felt_without_prefix() {
        let result = parse_felt("123abc");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_felt_empty() {
        let result = parse_felt("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_felt_invalid_chars() {
        let result = parse_felt("0xGHIJKL");
        assert!(result.is_err());
    }
}
