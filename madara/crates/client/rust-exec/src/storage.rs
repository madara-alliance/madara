//! Storage key computation utilities.
//!
//! This module provides functions to compute storage keys the same way
//! the Cairo compiler and runtime do.
//!
//! # Cairo 0 vs Cairo 1.0 Storage
//!
//! - **Cairo 0**: Uses Pedersen hash: `key = pedersen(base, address)`
//! - **Cairo 1.0**: Uses Poseidon hash: `key = poseidon(base, address)`

use sha3::{Digest, Keccak256};
pub use starknet_crypto::pedersen_hash;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

use crate::types::StorageKey;

/// Compute sn_keccak (Starknet's keccak variant).
///
/// This is used by the Cairo compiler to compute base addresses for storage variables.
/// The result is masked to 250 bits (Starknet's modification of standard keccak256).
///
/// # Example
/// ```
/// use mc_rust_exec::storage::sn_keccak;
/// let key = sn_keccak(b"balance");
/// ```
pub fn sn_keccak(data: &[u8]) -> Felt {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let mut result = hasher.finalize();

    // Mask to 250 bits (Starknet's modification)
    // Clear the top 6 bits of the first byte
    result[0] &= 0x03;

    Felt::from_bytes_be_slice(&result)
}

/// Compute storage key for a simple storage variable.
///
/// For a variable like `x: felt252`, the key is just `sn_keccak("x")`.
pub fn storage_key_for_variable(variable_name: &str) -> StorageKey {
    StorageKey(sn_keccak(variable_name.as_bytes()))
}

/// Compute storage key for a mapping with a single key.
///
/// For `mapping: Map<K, V>`, the storage key for `mapping[k]` is:
/// `pedersen(sn_keccak("mapping"), k)`
pub fn storage_key_for_map(variable_name: &str, key: Felt) -> StorageKey {
    let base = sn_keccak(variable_name.as_bytes());
    StorageKey(pedersen_hash(&base, &key))
}

/// Compute storage key for a mapping with two keys (nested mapping or tuple key).
///
/// For `mapping: Map<(K1, K2), V>`, the storage key for `mapping[(k1, k2)]` is:
/// `pedersen(sn_keccak("mapping"), pedersen(k1, k2))`
pub fn storage_key_for_map2(variable_name: &str, key1: Felt, key2: Felt) -> StorageKey {
    let base = sn_keccak(variable_name.as_bytes());
    let inner = pedersen_hash(&key1, &key2);
    StorageKey(pedersen_hash(&base, &inner))
}

// ============================================================================
// Cairo 1.0 Storage (Poseidon-based)
// ============================================================================

/// Compute Poseidon hash of multiple felts.
///
/// This is the hash function used by Cairo 1.0 for storage key computation.
pub fn poseidon_hash_many(values: &[Felt]) -> Felt {
    Poseidon::hash_array(values)
}

/// Compute storage key for a Cairo 1.0 mapping with a single key.
///
/// For Cairo 1.0 `mapping: LegacyMap<K, V>`, the storage key for `mapping[k]` is:
/// `poseidon(sn_keccak("mapping"), k)`
pub fn storage_key_for_map_poseidon(variable_name: &str, key: Felt) -> StorageKey {
    let base = sn_keccak(variable_name.as_bytes());
    StorageKey(Poseidon::hash_array(&[base, key]))
}

/// Try multiple variable names with Poseidon hash and return all computed keys.
///
/// This is useful for debugging to find which variable name matches Blockifier's keys.
pub fn try_balance_keys_poseidon(address: Felt) -> Vec<(&'static str, Felt)> {
    let variable_names = [
        "balances",
        "ERC20_balances",
        "ERC20Balances",
        "ERC20Component_balances",
        "ERC20_balance",
        "balance",
        "Balances",
        "token_balances",
    ];

    variable_names
        .iter()
        .map(|name| {
            let base = sn_keccak(name.as_bytes());
            let key = Poseidon::hash_array(&[base, address]);
            (*name, key)
        })
        .collect()
}

/// Compute event selector from event name.
///
/// Event selectors are computed as `sn_keccak(event_name)`.
pub fn event_selector(event_name: &str) -> Felt {
    sn_keccak(event_name.as_bytes())
}

/// Compute function selector from function name.
///
/// Function selectors are computed as `sn_keccak(function_name)`.
pub fn function_selector(function_name: &str) -> Felt {
    sn_keccak(function_name.as_bytes())
}

/// Convert a Cairo short string to a Felt.
///
/// Cairo short strings (up to 31 characters) are stored as ASCII bytes
/// packed into a felt, NOT as a keccak hash.
///
/// # Example
/// ```
/// use mc_rust_exec::storage::short_string_to_felt;
/// let felt = short_string_to_felt("hash_batch");
/// // This equals 0x686173685f6261746368 (ASCII bytes of "hash_batch")
/// ```
pub fn short_string_to_felt(s: &str) -> Felt {
    let bytes = s.as_bytes();
    assert!(bytes.len() <= 31, "Short string must be 31 characters or less");
    Felt::from_bytes_be_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sn_keccak_basic() {
        // The result should be a valid felt (< 2^251)
        let result = sn_keccak(b"test");
        // First byte should have top 6 bits cleared
        let bytes = result.to_bytes_be();
        assert!(bytes[0] <= 0x03);
    }

    #[test]
    fn test_storage_key_deterministic() {
        let key1 = storage_key_for_variable("balance");
        let key2 = storage_key_for_variable("balance");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_different_variables_different_keys() {
        let key1 = storage_key_for_variable("balance");
        let key2 = storage_key_for_variable("owner");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_counter_key_matches_blockifier() {
        // Blockifier's key from logs: 0x7ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854
        let expected_key = Felt::from_hex_unchecked("0x7ebcc807b5c7e19f245995a55aed6f46f5f582f476a886b91b834b0ddf5854");
        let rust_key = storage_key_for_variable("counter");
        println!("Expected key (Blockifier): {:#x}", expected_key);
        println!("Rust computed key:         {:#x}", rust_key.0);
        println!("Match: {}", rust_key.0 == expected_key);
        assert_eq!(rust_key.0, expected_key, "Counter key mismatch!");
    }
}
