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

use crate::contracts::paradex::{assets_manager, oracle, paraclear};
use crate::contracts::{account, erc20};

static PRECOMPUTED_SN_KECCAK_ENABLED: Lazy<bool> = Lazy::new(|| {
    let value = std::env::var("RUST_EXEC_PRECOMPUTED_SN_KECCAK").unwrap_or_default();
    if value.is_empty() {
        return false;
    }
    !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off")
});

static PRECOMPUTED_SN_KECCAK: Lazy<HashMap<&'static str, Felt>> = Lazy::new(|| {
    let mut map = HashMap::with_capacity(
        account::PRECOMPUTED_NAMES.len()
            + erc20::PRECOMPUTED_NAMES.len()
            + assets_manager::FUNCTION_NAMES.len()
            + assets_manager::PRECOMPUTED_NAMES.len()
            + oracle::FUNCTION_NAMES.len()
            + oracle::PRECOMPUTED_NAMES.len()
            + paraclear::FUNCTION_NAMES.len()
            + paraclear::PRECOMPUTED_NAMES.len(),
    );
    for value in account::PRECOMPUTED_NAMES
        .iter()
        .chain(erc20::PRECOMPUTED_NAMES)
        .chain(assets_manager::FUNCTION_NAMES)
        .chain(assets_manager::PRECOMPUTED_NAMES)
        .chain(oracle::FUNCTION_NAMES)
        .chain(oracle::PRECOMPUTED_NAMES)
        .chain(paraclear::FUNCTION_NAMES)
        .chain(paraclear::PRECOMPUTED_NAMES)
    {
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
