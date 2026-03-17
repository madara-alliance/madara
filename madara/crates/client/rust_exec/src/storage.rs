//! Storage key computation utilities.
//!
//! This module provides functions to compute storage keys the same way
//! the Cairo compiler and runtime do.
//!
//! # Cairo 0 vs Cairo 1.0 Storage
//!
//! - **Cairo 0**: Uses Pedersen hash: `key = pedersen(base, address)`
//! - **Cairo 1.0**: Uses Poseidon hash: `key = poseidon(base, address)`
//!
//! # Cairo storage formulas (reference)
//!
//! - Base address for a storage variable: `sn_keccak("var_name")`
//! - Map (single key): `pedersen(sn_keccak("map"), k)`
//! - Map (tuple key): `pedersen(sn_keccak("map"), pedersen(k1, k2))` (or chained pedersen)
//! - Storage node members: `pedersen(sn_keccak("var"), sn_keccak("member"))` (chain for nesting)
//! - Vec length at base; element at `pedersen(base, index)`

use once_cell::sync::Lazy;
use sha3::{Digest, Keccak256};
use starknet_crypto::pedersen_hash as pedersen_hash_inner;
use starknet_types_core::felt::{Felt, NonZeroFelt};
use starknet_types_core::hash::{Poseidon, StarkHash};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::contracts::paradex::precomputed_sn_keccak;
use crate::hash_agg;
use crate::types::StorageKey;
use keccak::f1600;

/// L2 address upper bound for storage keys (matches starknet_api::core).
static L2_ADDRESS_UPPER_BOUND: Lazy<NonZeroFelt> = Lazy::new(|| {
    // 2^251
    let two_pow_251 = Felt::from_hex_unchecked("0x800000000000000000000000000000000000000000000000000000000000000");
    // 2^251 - 256
    let bound = two_pow_251 - Felt::from(256u64);
    NonZeroFelt::try_from(bound).expect("L2_ADDRESS_UPPER_BOUND should be non-zero")
});

fn normalize_storage_address(value: Felt) -> Felt {
    value.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

/// Memoize Pedersen results because storage-key derivation repeatedly hashes the same (left,right)
/// pairs across settle_trade_v3 transactions.
///
/// Thread-local to avoid locking; capacity is bounded to avoid unbounded growth.
const PEDERSEN_CACHE_CAPACITY: usize = 8 * 1024;

thread_local! {
    static PEDERSEN_CACHE: RefCell<HashMap<(Felt, Felt), Felt>> = RefCell::new(HashMap::new());
}

#[derive(Default)]
struct KeyDerivationCache {
    map_base: HashMap<String, Felt>,
    map_key: HashMap<(Felt, Felt), StorageKey>,
    map2_key: HashMap<(Felt, Felt, Felt), StorageKey>,
    poseidon_map_key: HashMap<(Felt, Felt), StorageKey>,
    poseidon_map2_key: HashMap<(Felt, Felt, Felt), StorageKey>,
    substorage_map_poseidon: HashMap<(Felt, Felt, Felt), StorageKey>,
    substorage_map2_poseidon: HashMap<(Felt, Felt, Felt, Felt), StorageKey>,
    substorage_map_poseidon_add: HashMap<(Felt, Felt, Felt), StorageKey>,
    substorage_map2_poseidon_add: HashMap<(Felt, Felt, Felt, Felt), StorageKey>,
    substorage_map_poseidon_hash: HashMap<(Felt, Felt, Felt), StorageKey>,
    substorage_map2_poseidon_hash: HashMap<(Felt, Felt, Felt, Felt), StorageKey>,
    substorage_var_poseidon: HashMap<(Felt, Felt), StorageKey>,
    substorage_var_add: HashMap<(Felt, Felt), StorageKey>,
    node_member: HashMap<(Felt, Felt), StorageKey>,
}

impl KeyDerivationCache {
    fn clear(&mut self) {
        self.map_base.clear();
        self.map_key.clear();
        self.map2_key.clear();
        self.poseidon_map_key.clear();
        self.poseidon_map2_key.clear();
        self.substorage_map_poseidon.clear();
        self.substorage_map2_poseidon.clear();
        self.substorage_map_poseidon_add.clear();
        self.substorage_map2_poseidon_add.clear();
        self.substorage_map_poseidon_hash.clear();
        self.substorage_map2_poseidon_hash.clear();
        self.substorage_var_poseidon.clear();
        self.substorage_var_add.clear();
        self.node_member.clear();
    }
}

thread_local! {
    static KEY_DERIVATION_CACHE: RefCell<KeyDerivationCache> =
        RefCell::new(KeyDerivationCache::default());
}

pub fn reset_key_derivation_cache() {
    KEY_DERIVATION_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn map_base_cached(_kind: &str, variable_name: &str) -> Felt {
    if let Some(base) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map_base.get(variable_name).copied()) {
        hash_agg::record_key_cache_hit();
        return base;
    }
    hash_agg::record_key_cache_miss();
    let base = sn_keccak(variable_name.as_bytes());
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map_base.insert(variable_name.to_string(), base);
    });
    base
}

static PEDERSEN_CACHE_ENABLED: Lazy<bool> = Lazy::new(|| {
    std::env::var("RUST_EXEC_PEDERSEN_CACHE").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(true)
});

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
    if let Some(precomputed) = precomputed_sn_keccak::lookup_sn_keccak(data) {
        hash_agg::record_sn_keccak_call(data, true);
        return precomputed;
    }
    hash_agg::record_sn_keccak_call(data, false);
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let mut result = hasher.finalize();

    // Mask to 250 bits (Starknet's modification)
    // Clear the top 6 bits of the first byte
    result[0] &= 0x03;
    let felt = Felt::from_bytes_be_slice(&result);
    felt
}

/// Compute Cairo's keccak for u64 words and return the low/high u128 limbs.
///
/// Cairo's keccak works on u64 words, with standard keccak padding over the
/// 136-byte rate. Words are interpreted as little-endian byte lanes.
pub fn cairo_keccak_u64_words(words: &[u64], n_bytes: usize) -> (u128, u128) {
    const RATE_BYTES: usize = 136;
    const RATE_WORDS: usize = RATE_BYTES / 8;

    let mut state = [0u64; 25];
    let mut offset_bytes = 0usize;
    let mut word_index = 0usize;

    // Absorb full rate blocks.
    while n_bytes.saturating_sub(offset_bytes) >= RATE_BYTES {
        for i in 0..RATE_WORDS {
            state[i] ^= words[word_index + i];
        }
        f1600(&mut state);
        offset_bytes += RATE_BYTES;
        word_index += RATE_WORDS;
    }

    // Absorb final partial block.
    let remaining = n_bytes - offset_bytes;
    let full_words = remaining / 8;
    let remaining_bytes = remaining % 8;

    for i in 0..full_words {
        state[i] ^= words[word_index + i];
    }

    if remaining_bytes > 0 {
        let mask = (1u64 << (remaining_bytes * 8)) - 1;
        state[full_words] ^= words[word_index + full_words] & mask;
    }

    // Keccak padding: append 0x01, then 0x80 at the end of the rate.
    let pad_bit_offset = remaining_bytes * 8;
    state[full_words] ^= 1u64 << pad_bit_offset;
    state[RATE_WORDS - 1] ^= 1u64 << 63;

    f1600(&mut state);

    let low = (state[1] as u128) << 64 | state[0] as u128;
    let high = (state[3] as u128) << 64 | state[2] as u128;
    let out = (low, high);
    out
}

/// Compute Cairo's keccak for u64 words, mask to 250 bits, and return Felt.
pub fn cairo_keccak_u64_words_masked_felt(words: &[u64], n_bytes: usize) -> Felt {
    let (low, high) = cairo_keccak_u64_words(words, n_bytes);
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(&high.to_be_bytes());
    bytes[16..].copy_from_slice(&low.to_be_bytes());
    bytes[0] &= 0x03;
    Felt::from_bytes_be_slice(&bytes)
}

/// Compute storage key for a simple storage variable.
///
/// For a variable like `x: felt252`, the key is just `sn_keccak("x")`.
pub fn storage_key_for_variable(variable_name: &str) -> StorageKey {
    let base = map_base_cached("var", variable_name);
    StorageKey(normalize_storage_address(base))
}

/// Compute storage key for a mapping with a single key.
///
/// For `mapping: Map<K, V>`, the storage key for `mapping[k]` is:
/// `pedersen(sn_keccak("mapping"), k)`
pub fn storage_key_for_map(variable_name: &str, key: Felt) -> StorageKey {
    let base = map_base_cached("map_base", variable_name);
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map_key.get(&(base, key)).copied()) {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(pedersen_hash(&base, &key)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map_key.insert((base, key), out);
    });
    out
}

/// Compute storage key for a mapping with a single key using a precomputed base.
///
/// For `mapping: Map<K, V>`, the storage key for `mapping[k]` is:
/// `pedersen(base, k)` where `base = sn_keccak("mapping")`.
pub fn storage_key_for_map_with_base(base: Felt, key: Felt) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map_key.get(&(base, key)).copied()) {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(pedersen_hash(&base, &key)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map_key.insert((base, key), out);
    });
    out
}

/// Compute storage key for a mapping with a single key using a precomputed base (named).
pub fn storage_key_for_map_with_base_named(base: Felt, key: Felt, _name: &str) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map_key.get(&(base, key)).copied()) {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(pedersen_hash(&base, &key)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map_key.insert((base, key), out);
    });
    out
}

/// Compute storage key for a mapping with two keys (tuple key / nested mapping).
///
/// For `mapping: Map<(K1, K2), V>`, the storage key for `mapping[(k1, k2)]` is:
/// `pedersen(pedersen(sn_keccak("mapping"), k1), k2)`
pub fn storage_key_for_map2(variable_name: &str, key1: Felt, key2: Felt) -> StorageKey {
    let base = map_base_cached("map2_base", variable_name);
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map2_key.get(&(base, key1, key2)).copied()) {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let inner = pedersen_hash(&base, &key1);
    let out = StorageKey(normalize_storage_address(pedersen_hash(&inner, &key2)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map2_key.insert((base, key1, key2), out);
    });
    out
}

/// Compute storage key for a mapping with two keys using a precomputed base.
///
/// For `mapping: Map<(K1, K2), V>`, the storage key is:
/// `pedersen(pedersen(base, k1), k2)` where `base = sn_keccak("mapping")`.
pub fn storage_key_for_map2_with_base(base: Felt, key1: Felt, key2: Felt) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map2_key.get(&(base, key1, key2)).copied()) {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let inner = pedersen_hash(&base, &key1);
    let out = StorageKey(normalize_storage_address(pedersen_hash(&inner, &key2)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map2_key.insert((base, key1, key2), out);
    });
    out
}

/// Compute storage key for a mapping with two keys using a precomputed base (named).
pub fn storage_key_for_map2_with_base_named(base: Felt, key1: Felt, key2: Felt, _name: &str) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().map2_key.get(&(base, key1, key2)).copied()) {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let inner = pedersen_hash(&base, &key1);
    let out = StorageKey(normalize_storage_address(pedersen_hash(&inner, &key2)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().map2_key.insert((base, key1, key2), out);
    });
    out
}

/// Compute storage key for a storage node member (legacy pedersen layout).
///
/// For a node `node` with member `member`, the base is:
/// `pedersen(sn_keccak("node"), sn_keccak("member"))`
pub fn storage_key_for_storage_node_member_pedersen(node: &str, member: &str) -> StorageKey {
    let node_key = map_base_cached("node_base", node);
    let member_key = map_base_cached("node_member", member);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().node_member.get(&(node_key, member_key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(pedersen_hash(&node_key, &member_key)));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().node_member.insert((node_key, member_key), out);
    });
    out
}

// ============================================================================
// Cairo 1.0 Storage (Poseidon-based)
// ============================================================================

/// Compute Poseidon hash of multiple felts.
///
/// This is the hash function used by Cairo 1.0 for storage key computation.
pub fn poseidon_hash_many(values: &[Felt]) -> Felt {
    hash_agg::record_poseidon_call(values);
    let out = Poseidon::hash_array(values);
    out
}

pub fn pedersen_hash(left: &Felt, right: &Felt) -> Felt {
    hash_agg::record_pedersen_call(*left, *right);
    if *PEDERSEN_CACHE_ENABLED {
        // Fast path: cache hit. Don't count this as hashing time.
        if let Some(value) = PEDERSEN_CACHE.with(|cache| cache.borrow().get(&(*left, *right)).copied()) {
            hash_agg::record_pedersen_cache_hit();
            return value;
        }
        hash_agg::record_pedersen_cache_miss();
    } else {
        hash_agg::record_pedersen_cache_miss();
    }

    // Cache miss: do the expensive hashing and account it in the hashing timer.
    let out = pedersen_hash_inner(left, right);

    if *PEDERSEN_CACHE_ENABLED {
        PEDERSEN_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= PEDERSEN_CACHE_CAPACITY {
                // Simple bounded policy: clear when reaching capacity. In practice the hot-set for
                // settle_trade_v3 is tiny (hundreds of pairs), so this shouldn't thrash.
                cache.clear();
            }
            cache.insert((*left, *right), out);
        });
    }

    out
}

/// Compute storage key for a Cairo 1.0 mapping with a single key.
///
/// For Cairo 1.0 `mapping: LegacyMap<K, V>`, the storage key for `mapping[k]` is:
/// `poseidon(sn_keccak("mapping"), k)`
pub fn storage_key_for_map_poseidon(variable_name: &str, key: Felt) -> StorageKey {
    let base = map_base_cached("poseidon_map_base", variable_name);
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().poseidon_map_key.get(&(base, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().poseidon_map_key.insert((base, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 mapping with a single key using a precomputed base.
pub fn storage_key_for_map_poseidon_with_base(base: Felt, key: Felt) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().poseidon_map_key.get(&(base, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().poseidon_map_key.insert((base, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 mapping with a single key using a precomputed base (named).
pub fn storage_key_for_map_poseidon_with_base_named(base: Felt, key: Felt, _name: &str) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE.with(|cache| cache.borrow().poseidon_map_key.get(&(base, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().poseidon_map_key.insert((base, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 mapping with two keys.
///
/// For Cairo 1.0 `mapping: Map<(K1, K2), V>`, the storage key is:
/// `poseidon(sn_keccak("mapping"), k1, k2)`
pub fn storage_key_for_map2_poseidon(variable_name: &str, key1: Felt, key2: Felt) -> StorageKey {
    let base = map_base_cached("poseidon_map2_base", variable_name);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().poseidon_map2_key.get(&(base, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().poseidon_map2_key.insert((base, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 mapping with two keys using a precomputed base.
pub fn storage_key_for_map2_poseidon_with_base(base: Felt, key1: Felt, key2: Felt) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().poseidon_map2_key.get(&(base, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().poseidon_map2_key.insert((base, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 mapping with two keys using a precomputed base (named).
pub fn storage_key_for_map2_poseidon_with_base_named(base: Felt, key1: Felt, key2: Felt, _name: &str) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().poseidon_map2_key.get(&(base, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().poseidon_map2_key.insert((base, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map with a single key.
///
/// For `#[substorage] component: Storage { mapping: Map<K, V> }`, the key is:
/// `poseidon(component_base, sn_keccak("mapping"), k)`
pub fn storage_key_for_substorage_map_poseidon(base: StorageKey, variable_name: &str, key: Felt) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map using a precomputed var.
pub fn storage_key_for_substorage_map_poseidon_with_var(base: StorageKey, var: Felt, key: Felt) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map using a precomputed var (named).
pub fn storage_key_for_substorage_map_poseidon_with_var_named(
    base: StorageKey,
    var: Felt,
    key: Felt,
    _name: &str,
) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map using base+var then poseidon(base, key).
///
/// Some storage paths use `storage_address_from_base`, which is effectively base + sn_keccak(var),
/// before applying the map hash. This helper tries that layout.
pub fn storage_key_for_substorage_map_poseidon_add(base: StorageKey, variable_name: &str, key: Felt) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon_add.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = base.0 + var;
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon_add.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map (base+var) using a precomputed var.
pub fn storage_key_for_substorage_map_poseidon_add_with_var(base: StorageKey, var: Felt, key: Felt) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon_add.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = base.0 + var;
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon_add.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map (base+var) using a precomputed var (named).
pub fn storage_key_for_substorage_map_poseidon_add_with_var_named(
    base: StorageKey,
    var: Felt,
    key: Felt,
    _name: &str,
) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon_add.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = base.0 + var;
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon_add.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map using poseidon(base, var) then poseidon(base, key).
pub fn storage_key_for_substorage_map_poseidon_hash(base: StorageKey, variable_name: &str, key: Felt) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon_hash.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = poseidon_hash_many(&[base.0, var]);
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon_hash.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map (poseidon(base,var)) using a precomputed var.
pub fn storage_key_for_substorage_map_poseidon_hash_with_var(base: StorageKey, var: Felt, key: Felt) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon_hash.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = poseidon_hash_many(&[base.0, var]);
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon_hash.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map (poseidon(base,var)) using a precomputed var (named).
pub fn storage_key_for_substorage_map_poseidon_hash_with_var_named(
    base: StorageKey,
    var: Felt,
    key: Felt,
    _name: &str,
) -> StorageKey {
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_map_poseidon_hash.get(&(base.0, var, key)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = poseidon_hash_many(&[base.0, var]);
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map_poseidon_hash.insert((base.0, var, key), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map with two keys.
///
/// For `#[substorage] component: Storage { mapping: Map<(K1, K2), V> }`, the key is:
/// `poseidon(component_base, sn_keccak("mapping"), k1, k2)`
pub fn storage_key_for_substorage_map2_poseidon(
    base: StorageKey,
    variable_name: &str,
    key1: Felt,
    key2: Felt,
) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 using a precomputed var.
pub fn storage_key_for_substorage_map2_poseidon_with_var(
    base: StorageKey,
    var: Felt,
    key1: Felt,
    key2: Felt,
) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 using a precomputed var (named).
pub fn storage_key_for_substorage_map2_poseidon_with_var_named(
    base: StorageKey,
    var: Felt,
    key1: Felt,
    key2: Felt,
    _name: &str,
) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 using base+var then poseidon(base, k1, k2).
pub fn storage_key_for_substorage_map2_poseidon_add(
    base: StorageKey,
    variable_name: &str,
    key1: Felt,
    key2: Felt,
) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon_add.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = base.0 + var;
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon_add.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 (base+var) using a precomputed var.
pub fn storage_key_for_substorage_map2_poseidon_add_with_var(
    base: StorageKey,
    var: Felt,
    key1: Felt,
    key2: Felt,
) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon_add.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = base.0 + var;
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon_add.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 (base+var) using a precomputed var (named).
pub fn storage_key_for_substorage_map2_poseidon_add_with_var_named(
    base: StorageKey,
    var: Felt,
    key1: Felt,
    key2: Felt,
    _name: &str,
) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon_add.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = base.0 + var;
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon_add.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 using poseidon(base, var) then poseidon(base, k1, k2).
pub fn storage_key_for_substorage_map2_poseidon_hash(
    base: StorageKey,
    variable_name: &str,
    key1: Felt,
    key2: Felt,
) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon_hash.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = poseidon_hash_many(&[base.0, var]);
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon_hash.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 (poseidon(base,var)) using a precomputed var.
pub fn storage_key_for_substorage_map2_poseidon_hash_with_var(
    base: StorageKey,
    var: Felt,
    key1: Felt,
    key2: Felt,
) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon_hash.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = poseidon_hash_many(&[base.0, var]);
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon_hash.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage map2 (poseidon(base,var)) using a precomputed var (named).
pub fn storage_key_for_substorage_map2_poseidon_hash_with_var_named(
    base: StorageKey,
    var: Felt,
    key1: Felt,
    key2: Felt,
    _name: &str,
) -> StorageKey {
    if let Some(cached) = KEY_DERIVATION_CACHE
        .with(|cache| cache.borrow().substorage_map2_poseidon_hash.get(&(base.0, var, key1, key2)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let base_field = poseidon_hash_many(&[base.0, var]);
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base_field, key1, key2])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_map2_poseidon_hash.insert((base.0, var, key1, key2), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage variable using poseidon(base, var).
pub fn storage_key_for_substorage_var_poseidon(base: StorageKey, variable_name: &str) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_var_poseidon.get(&(base.0, var)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(poseidon_hash_many(&[base.0, var])));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_var_poseidon.insert((base.0, var), out);
    });
    out
}

/// Compute storage key for a Cairo 1.0 substorage variable using base + var.
pub fn storage_key_for_substorage_var_add(base: StorageKey, variable_name: &str) -> StorageKey {
    let var = map_base_cached("poseidon_substorage_var", variable_name);
    if let Some(cached) =
        KEY_DERIVATION_CACHE.with(|cache| cache.borrow().substorage_var_add.get(&(base.0, var)).copied())
    {
        hash_agg::record_key_cache_hit();
        return cached;
    }
    hash_agg::record_key_cache_miss();
    let out = StorageKey(normalize_storage_address(base.0 + var));
    KEY_DERIVATION_CACHE.with(|cache| {
        cache.borrow_mut().substorage_var_add.insert((base.0, var), out);
    });
    out
}

/// Add a small offset to a storage key (for struct field slots).
pub fn storage_key_with_offset(base: StorageKey, offset: u8) -> StorageKey {
    let mut bytes = base.0.to_bytes_be();
    let mut value = Felt::from_bytes_be_slice(&bytes);
    value += Felt::from(offset as u64);
    bytes = value.to_bytes_be();
    StorageKey(Felt::from_bytes_be_slice(&bytes))
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

    #[test]
    fn test_cairo_keccak_word_encoding_example() {
        // Example from cairo-vm/cairo_programs/cairo_finalize_keccak.cairo
        let input = [8031924123371070792_u64, 560229490_u64];
        let expected_low: u128 = "293431514620200399776069983710520819074".parse().unwrap();
        let expected_high: u128 = "317109767021952548743448767588473366791".parse().unwrap();

        let (low, high) = cairo_keccak_u64_words(&input, 16);
        assert_eq!(low, expected_low);
        assert_eq!(high, expected_high);
    }
}
