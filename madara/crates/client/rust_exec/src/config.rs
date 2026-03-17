//! Configuration for Rust native execution.
//!
//! The current pipeline initializes RustExec explicitly from block production at startup.
//! Environment variables remain as a fallback for standalone use and tests, but they are no
//! longer the primary runtime configuration surface.

use once_cell::sync::{Lazy, OnceCell};
use starknet_types_core::felt::Felt;
use std::env;

/// Environment variable name for Account class hash
pub const ENV_ACCOUNT_CLASS_HASH: &str = "RUST_EXEC_ACCOUNT_CLASS_HASH";

/// Environment variable name for ERC20 class hash
pub const ENV_ERC20_CLASS_HASH: &str = "RUST_EXEC_ERC20_CLASS_HASH";

/// Environment variable name for Paraclear class hash
pub const ENV_PARACLEAR_CLASS_HASH: &str = "RUST_EXEC_PARACLEAR_CLASS_HASH";

/// Environment variable name for Paraclear Oracle class hash
pub const ENV_PARACLEAR_ORACLE_CLASS_HASH: &str = "RUST_EXEC_PARACLEAR_ORACLE_CLASS_HASH";

/// Environment variable name for AssetsManager class hash
pub const ENV_ASSETS_MANAGER_CLASS_HASH: &str = "RUST_EXEC_ASSETS_MANAGER_CLASS_HASH";

/// Parse a hex string into a Felt.
///
/// Accepts formats:
/// - `0x123abc...` (with prefix)
/// - `123abc...` (without prefix)
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
    pub settle_trade_fixed_fee: u128,
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
            settle_trade_fixed_fee: crate::constants::SETTLE_TRADE_V3_FIXED_FEE_AMOUNT,
        }
    }
}

static RUNTIME_CONFIG: OnceCell<RustExecRuntimeConfig> = OnceCell::new();

/// Parsed class hash for Account, read from environment at startup.
pub static ACCOUNT_CLASS_HASH: Lazy<Option<Felt>> = Lazy::new(|| match env::var(ENV_ACCOUNT_CLASS_HASH) {
    Ok(value) => {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        match parse_felt(trimmed) {
            Ok(felt) => Some(felt),
            Err(_e) => None,
        }
    }
    Err(_) => None,
});

pub fn account_class_hash() -> Option<Felt> {
    runtime_config().map(|cfg| cfg.account_class_hash).or(*ACCOUNT_CLASS_HASH)
}

/// Parsed class hash for ERC20, read from environment at startup.
pub static ERC20_CLASS_HASH: Lazy<Option<Felt>> = Lazy::new(|| match env::var(ENV_ERC20_CLASS_HASH) {
    Ok(value) => {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        match parse_felt(trimmed) {
            Ok(felt) => Some(felt),
            Err(_e) => None,
        }
    }
    Err(_) => None,
});

pub fn erc20_class_hash() -> Option<Felt> {
    runtime_config().map(|cfg| cfg.erc20_class_hash).or(*ERC20_CLASS_HASH)
}

/// Parsed class hash for Paraclear, read from environment at startup.
pub static PARACLEAR_CLASS_HASH: Lazy<Option<Felt>> = Lazy::new(|| match env::var(ENV_PARACLEAR_CLASS_HASH) {
    Ok(value) => {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        match parse_felt(trimmed) {
            Ok(felt) => Some(felt),
            Err(_e) => None,
        }
    }
    Err(_) => None,
});

pub fn paraclear_class_hash() -> Option<Felt> {
    runtime_config().map(|cfg| cfg.paraclear_class_hash).or(*PARACLEAR_CLASS_HASH)
}

/// Parsed class hash for Paraclear Oracle, read from environment at startup.
pub static PARACLEAR_ORACLE_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_PARACLEAR_ORACLE_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => Some(felt),
                Err(_e) => None,
            }
        }
        Err(_) => None,
    });

pub fn paraclear_oracle_class_hash() -> Option<Felt> {
    runtime_config().map(|cfg| cfg.paraclear_oracle_class_hash).or(*PARACLEAR_ORACLE_CLASS_HASH)
}

/// Parsed class hash for AssetsManager, read from environment at startup.
pub static ASSETS_MANAGER_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_ASSETS_MANAGER_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => Some(felt),
                Err(_e) => None,
            }
        }
        Err(_) => None,
    });

pub fn assets_manager_class_hash() -> Option<Felt> {
    runtime_config().map(|cfg| cfg.assets_manager_class_hash).or(*ASSETS_MANAGER_CLASS_HASH)
}

pub fn runtime_config() -> Option<&'static RustExecRuntimeConfig> {
    RUNTIME_CONFIG.get()
}

pub fn initialize_runtime_config(config: RustExecRuntimeConfig) {
    let _ = RUNTIME_CONFIG.set(config);
}

/// Check if Rust verification is enabled for any contracts.
///
/// Returns `true` if at least one contract class hash is configured.
pub fn is_verification_enabled() -> bool {
    ACCOUNT_CLASS_HASH.is_some()
        || ERC20_CLASS_HASH.is_some()
        || PARACLEAR_CLASS_HASH.is_some()
        || PARACLEAR_ORACLE_CLASS_HASH.is_some()
        || ASSETS_MANAGER_CLASS_HASH.is_some()
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
