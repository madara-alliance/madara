//! Configuration for Rust native execution.
//!
//! Class hashes are read from environment variables, allowing dynamic configuration
//! without recompilation.
//!
//! # Environment Variables
//!
//! | Variable | Description |
//! |----------|-------------|
//! | `RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH` | Class hash for SimpleCounter contract |
//! | `RUST_EXEC_COUNTER_WITH_EVENT_CLASS_HASH` | Class hash for CounterWithEvent contract |
//! | `RUST_EXEC_RANDOM_100_HASHES_CLASS_HASH` | Class hash for Random100Hashes contract |
//! | `RUST_EXEC_MATH_BENCHMARK_CLASS_HASH` | Class hash for MathBenchmark contract |
//!
//! # Usage
//!
//! Set in `.env` file or export directly:
//! ```bash
//! export RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH=0x0123456789abcdef...
//! export RUST_EXEC_COUNTER_WITH_EVENT_CLASS_HASH=0x0123456789abcdef...
//! ```
//!
//! Then the rust-exec verification will automatically match transactions
//! calling contracts with this class hash.

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;
use std::env;

/// Environment variable name for SimpleCounter class hash
pub const ENV_SIMPLE_COUNTER_CLASS_HASH: &str = "RUST_EXEC_SIMPLE_COUNTER_CLASS_HASH";

/// Environment variable name for CounterWithEvent class hash
pub const ENV_COUNTER_WITH_EVENT_CLASS_HASH: &str = "RUST_EXEC_COUNTER_WITH_EVENT_CLASS_HASH";

/// Environment variable name for Random100Hashes class hash
pub const ENV_RANDOM_100_HASHES_CLASS_HASH: &str = "RUST_EXEC_RANDOM_100_HASHES_CLASS_HASH";

/// Environment variable name for MathBenchmark class hash
pub const ENV_MATH_BENCHMARK_CLASS_HASH: &str = "RUST_EXEC_MATH_BENCHMARK_CLASS_HASH";

/// Environment variable name for Account class hash
pub const ENV_ACCOUNT_CLASS_HASH: &str = "RUST_EXEC_ACCOUNT_CLASS_HASH";

/// Environment variable name for ERC20 class hash
pub const ENV_ERC20_CLASS_HASH: &str = "RUST_EXEC_ERC20_CLASS_HASH";

/// Environment variable name for HeavyTradeSimulator class hash
pub const ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH: &str = "RUST_EXEC_HEAVY_TRADE_SIMULATOR_CLASS_HASH";

/// Environment variable name for StorageHeavy class hash
pub const ENV_STORAGE_HEAVY_CLASS_HASH: &str = "RUST_EXEC_STORAGE_HEAVY_CLASS_HASH";

/// Environment variable name for ComprehensiveBenchmark class hash
pub const ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH: &str = "RUST_EXEC_COMPREHENSIVE_BENCHMARK_CLASS_HASH";

/// Parsed class hash for SimpleCounter, read from environment at startup.
///
/// If the environment variable is not set or invalid, this will be `None`
/// and the contract will not be matched for Rust verification.
pub static SIMPLE_COUNTER_CLASS_HASH: Lazy<Option<Felt>> = Lazy::new(|| {
    match env::var(ENV_SIMPLE_COUNTER_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, SimpleCounter Rust verification disabled",
                    ENV_SIMPLE_COUNTER_CLASS_HASH
                );
                return None;
            }

            // Parse the hex string (with or without 0x prefix)
            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for SimpleCounter with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. SimpleCounter Rust verification disabled.",
                        ENV_SIMPLE_COUNTER_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, SimpleCounter Rust verification disabled",
                ENV_SIMPLE_COUNTER_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, SimpleCounter Rust verification disabled",
                ENV_SIMPLE_COUNTER_CLASS_HASH
            );
            None
        }
    }
});

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

/// Get the configured class hash for SimpleCounter.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn simple_counter_class_hash() -> Option<Felt> {
    *SIMPLE_COUNTER_CLASS_HASH
}

/// Parsed class hash for CounterWithEvent, read from environment at startup.
pub static COUNTER_WITH_EVENT_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_COUNTER_WITH_EVENT_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, CounterWithEvent Rust verification disabled",
                    ENV_COUNTER_WITH_EVENT_CLASS_HASH
                );
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for CounterWithEvent with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. CounterWithEvent Rust verification disabled.",
                        ENV_COUNTER_WITH_EVENT_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, CounterWithEvent Rust verification disabled",
                ENV_COUNTER_WITH_EVENT_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, CounterWithEvent Rust verification disabled",
                ENV_COUNTER_WITH_EVENT_CLASS_HASH
            );
            None
        }
    });

/// Get the configured class hash for CounterWithEvent.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn counter_with_event_class_hash() -> Option<Felt> {
    *COUNTER_WITH_EVENT_CLASS_HASH
}

/// Parsed class hash for Random100Hashes, read from environment at startup.
pub static RANDOM_100_HASHES_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_RANDOM_100_HASHES_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, Random100Hashes Rust verification disabled",
                    ENV_RANDOM_100_HASHES_CLASS_HASH
                );
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for Random100Hashes with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. Random100Hashes Rust verification disabled.",
                        ENV_RANDOM_100_HASHES_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, Random100Hashes Rust verification disabled",
                ENV_RANDOM_100_HASHES_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, Random100Hashes Rust verification disabled",
                ENV_RANDOM_100_HASHES_CLASS_HASH
            );
            None
        }
    });

/// Get the configured class hash for Random100Hashes.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn random_100_hashes_class_hash() -> Option<Felt> {
    *RANDOM_100_HASHES_CLASS_HASH
}

/// Parsed class hash for MathBenchmark, read from environment at startup.
pub static MATH_BENCHMARK_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_MATH_BENCHMARK_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, MathBenchmark Rust verification disabled",
                    ENV_MATH_BENCHMARK_CLASS_HASH
                );
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for MathBenchmark with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. MathBenchmark Rust verification disabled.",
                        ENV_MATH_BENCHMARK_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, MathBenchmark Rust verification disabled",
                ENV_MATH_BENCHMARK_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, MathBenchmark Rust verification disabled",
                ENV_MATH_BENCHMARK_CLASS_HASH
            );
            None
        }
    });

/// Get the configured class hash for MathBenchmark.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn math_benchmark_class_hash() -> Option<Felt> {
    *MATH_BENCHMARK_CLASS_HASH
}

/// Parsed class hash for Account contract, read from environment at startup.
pub static ACCOUNT_CLASS_HASH: Lazy<Option<Felt>> = Lazy::new(|| match env::var(ENV_ACCOUNT_CLASS_HASH) {
    Ok(value) => {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            tracing::debug!(
                "Environment variable {} is empty, Account Rust verification disabled",
                ENV_ACCOUNT_CLASS_HASH
            );
            return None;
        }

        match parse_felt(trimmed) {
            Ok(felt) => {
                tracing::info!("Rust verification enabled for Account with class hash: {:#x}", felt);
                Some(felt)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to parse {} value '{}': {}. Account Rust verification disabled.",
                    ENV_ACCOUNT_CLASS_HASH,
                    trimmed,
                    e
                );
                None
            }
        }
    }
    Err(env::VarError::NotPresent) => {
        tracing::debug!("Environment variable {} not set, Account Rust verification disabled", ENV_ACCOUNT_CLASS_HASH);
        None
    }
    Err(env::VarError::NotUnicode(_)) => {
        tracing::warn!(
            "Environment variable {} contains invalid unicode, Account Rust verification disabled",
            ENV_ACCOUNT_CLASS_HASH
        );
        None
    }
});

/// Get the configured class hash for Account.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn account_class_hash() -> Option<Felt> {
    *ACCOUNT_CLASS_HASH
}

/// Parsed class hash for ERC20 contract, read from environment at startup.
pub static ERC20_CLASS_HASH: Lazy<Option<Felt>> = Lazy::new(|| match env::var(ENV_ERC20_CLASS_HASH) {
    Ok(value) => {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            tracing::debug!("Environment variable {} is empty, ERC20 Rust verification disabled", ENV_ERC20_CLASS_HASH);
            return None;
        }

        match parse_felt(trimmed) {
            Ok(felt) => {
                tracing::info!("Rust verification enabled for ERC20 with class hash: {:#x}", felt);
                Some(felt)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to parse {} value '{}': {}. ERC20 Rust verification disabled.",
                    ENV_ERC20_CLASS_HASH,
                    trimmed,
                    e
                );
                None
            }
        }
    }
    Err(env::VarError::NotPresent) => {
        tracing::debug!("Environment variable {} not set, ERC20 Rust verification disabled", ENV_ERC20_CLASS_HASH);
        None
    }
    Err(env::VarError::NotUnicode(_)) => {
        tracing::warn!(
            "Environment variable {} contains invalid unicode, ERC20 Rust verification disabled",
            ENV_ERC20_CLASS_HASH
        );
        None
    }
});

/// Get the configured class hash for ERC20.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn erc20_class_hash() -> Option<Felt> {
    *ERC20_CLASS_HASH
}

/// Parsed class hash for HeavyTradeSimulator contract, read from environment at startup.
pub static HEAVY_TRADE_SIMULATOR_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, HeavyTradeSimulator Rust verification disabled",
                    ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH
                );
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for HeavyTradeSimulator with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. HeavyTradeSimulator Rust verification disabled.",
                        ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, HeavyTradeSimulator Rust verification disabled",
                ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, HeavyTradeSimulator Rust verification disabled",
                ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH
            );
            None
        }
    });

/// Get the configured class hash for HeavyTradeSimulator.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn heavy_trade_simulator_class_hash() -> Option<Felt> {
    *HEAVY_TRADE_SIMULATOR_CLASS_HASH
}

/// Parsed class hash for StorageHeavy contract, read from environment at startup.
pub static STORAGE_HEAVY_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_STORAGE_HEAVY_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, StorageHeavy Rust verification disabled",
                    ENV_STORAGE_HEAVY_CLASS_HASH
                );
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for StorageHeavy with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. StorageHeavy Rust verification disabled.",
                        ENV_STORAGE_HEAVY_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, StorageHeavy Rust verification disabled",
                ENV_STORAGE_HEAVY_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, StorageHeavy Rust verification disabled",
                ENV_STORAGE_HEAVY_CLASS_HASH
            );
            None
        }
    });

/// Get the configured class hash for StorageHeavy.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn storage_heavy_class_hash() -> Option<Felt> {
    *STORAGE_HEAVY_CLASS_HASH
}

/// Parsed class hash for ComprehensiveBenchmark contract, read from environment at startup.
pub static COMPREHENSIVE_BENCHMARK_CLASS_HASH: Lazy<Option<Felt>> =
    Lazy::new(|| match env::var(ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                tracing::debug!(
                    "Environment variable {} is empty, ComprehensiveBenchmark Rust verification disabled",
                    ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH
                );
                return None;
            }

            match parse_felt(trimmed) {
                Ok(felt) => {
                    tracing::info!("Rust verification enabled for ComprehensiveBenchmark with class hash: {:#x}", felt);
                    Some(felt)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse {} value '{}': {}. ComprehensiveBenchmark Rust verification disabled.",
                        ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH,
                        trimmed,
                        e
                    );
                    None
                }
            }
        }
        Err(env::VarError::NotPresent) => {
            tracing::debug!(
                "Environment variable {} not set, ComprehensiveBenchmark Rust verification disabled",
                ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH
            );
            None
        }
        Err(env::VarError::NotUnicode(_)) => {
            tracing::warn!(
                "Environment variable {} contains invalid unicode, ComprehensiveBenchmark Rust verification disabled",
                ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH
            );
            None
        }
    });

/// Get the configured class hash for ComprehensiveBenchmark.
///
/// Returns `None` if the environment variable is not set or invalid.
pub fn comprehensive_benchmark_class_hash() -> Option<Felt> {
    *COMPREHENSIVE_BENCHMARK_CLASS_HASH
}

/// Check if Rust verification is enabled for any contracts.
///
/// Returns `true` if at least one contract class hash is configured.
pub fn is_verification_enabled() -> bool {
    SIMPLE_COUNTER_CLASS_HASH.is_some()
        || COUNTER_WITH_EVENT_CLASS_HASH.is_some()
        || RANDOM_100_HASHES_CLASS_HASH.is_some()
        || MATH_BENCHMARK_CLASS_HASH.is_some()
        || ACCOUNT_CLASS_HASH.is_some()
        || ERC20_CLASS_HASH.is_some()
        || HEAVY_TRADE_SIMULATOR_CLASS_HASH.is_some()
        || STORAGE_HEAVY_CLASS_HASH.is_some()
        || COMPREHENSIVE_BENCHMARK_CLASS_HASH.is_some()
}

/// Log the current configuration status.
pub fn log_config_status() {
    if is_verification_enabled() {
        tracing::info!("Rust execution verification is ENABLED");
        if let Some(hash) = simple_counter_class_hash() {
            tracing::info!("  - SimpleCounter: {:#x}", hash);
        }
        if let Some(hash) = counter_with_event_class_hash() {
            tracing::info!("  - CounterWithEvent: {:#x}", hash);
        }
        if let Some(hash) = random_100_hashes_class_hash() {
            tracing::info!("  - Random100Hashes: {:#x}", hash);
        }
        if let Some(hash) = math_benchmark_class_hash() {
            tracing::info!("  - MathBenchmark: {:#x}", hash);
        }
        if let Some(hash) = account_class_hash() {
            tracing::info!("  - Account: {:#x}", hash);
        }
        if let Some(hash) = erc20_class_hash() {
            tracing::info!("  - ERC20: {:#x}", hash);
        }
        if let Some(hash) = heavy_trade_simulator_class_hash() {
            tracing::info!("  - HeavyTradeSimulator: {:#x}", hash);
        }
        if let Some(hash) = storage_heavy_class_hash() {
            tracing::info!("  - StorageHeavy: {:#x}", hash);
        }
        if let Some(hash) = comprehensive_benchmark_class_hash() {
            tracing::info!("  - ComprehensiveBenchmark: {:#x}", hash);
        }
    } else {
        tracing::info!(
            "Rust execution verification is DISABLED (no class hashes configured). \
             Set {}, {}, {}, {}, {}, {}, {}, {}, or {} to enable.",
            ENV_SIMPLE_COUNTER_CLASS_HASH,
            ENV_COUNTER_WITH_EVENT_CLASS_HASH,
            ENV_RANDOM_100_HASHES_CLASS_HASH,
            ENV_MATH_BENCHMARK_CLASS_HASH,
            ENV_ACCOUNT_CLASS_HASH,
            ENV_ERC20_CLASS_HASH,
            ENV_HEAVY_TRADE_SIMULATOR_CLASS_HASH,
            ENV_STORAGE_HEAVY_CLASS_HASH,
            ENV_COMPREHENSIVE_BENCHMARK_CLASS_HASH
        );
    }
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
