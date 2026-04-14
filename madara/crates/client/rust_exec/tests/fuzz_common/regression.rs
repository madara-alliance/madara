//! Regression capture and replay for fuzz test failures.
//!
//! When a differential fuzz test finds a divergence, the full test case is serialized
//! to JSON so it can be replayed deterministically as a regression test.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::warn;

/// A captured fuzz test failure that can be replayed deterministically.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegressionCase {
    /// Human-readable label (e.g., "settle_trade_v3_divergence")
    pub label: String,
    /// Storage state entries: (contract_hex, key_hex, value_hex)
    pub storage: Vec<(String, String, String)>,
    /// Nonce entries: (contract_hex, nonce_hex)
    pub nonces: Vec<(String, String)>,
    /// Class hash entries: (contract_hex, class_hash_hex)
    pub class_hashes: Vec<(String, String)>,
    /// The calldata as hex-encoded Felts
    pub calldata: Vec<String>,
    /// Contract address under test (hex)
    pub contract_address: String,
    /// Block timestamp used
    pub block_timestamp: u64,
    /// Description of errors found
    pub errors: Vec<String>,
}

/// Save a regression case to disk.
///
/// Creates the directory if it doesn't exist. The file is named by a hash of the
/// calldata to avoid duplicates.
pub fn save_regression(test_name: &str, case: &RegressionCase) {
    let dir = format!("proptest-regressions/{}", test_name);
    if std::fs::create_dir_all(&dir).is_err() {
        warn!("could not create regression directory {}", dir);
        return;
    }

    // Use a simple hash of the calldata for the filename
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        case.calldata.hash(&mut hasher);
        case.block_timestamp.hash(&mut hasher);
        hasher.finish()
    };

    let path = format!("{}/{:016x}.json", dir, hash);
    match serde_json::to_string_pretty(case) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("could not write regression file {}: {}", path, e);
            }
        }
        Err(e) => {
            warn!("could not serialize regression case: {}", e);
        }
    }
}

/// Load all regression cases from a directory.
pub fn load_regressions(test_name: &str) -> Vec<RegressionCase> {
    let dir = format!("proptest-regressions/{}", test_name);
    let path = Path::new(&dir);
    if !path.exists() {
        return Vec::new();
    }

    let mut cases = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(contents) = std::fs::read_to_string(entry.path()) {
                    match serde_json::from_str::<RegressionCase>(&contents) {
                        Ok(case) => cases.push(case),
                        Err(e) => {
                            warn!("could not parse {}: {}", entry.path().display(), e);
                        }
                    }
                }
            }
        }
    }
    cases
}
