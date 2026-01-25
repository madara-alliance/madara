//! Storage key computation utilities.
//!
//! This module provides functions to compute storage keys the same way
//! the Cairo compiler and runtime do.

use sha3::{Digest, Keccak256};
use starknet_crypto::pedersen_hash;
use starknet_types_core::felt::Felt;

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
}
