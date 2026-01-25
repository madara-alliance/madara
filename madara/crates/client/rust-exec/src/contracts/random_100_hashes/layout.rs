//! Storage layout for Random100Hashes contract.

use once_cell::sync::Lazy;

use crate::storage::storage_key_for_variable;
use crate::types::StorageKey;

/// Storage key for `last_hash_result: felt252`.
///
/// Computed as `sn_keccak("last_hash_result")`.
pub static LAST_HASH_RESULT_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("last_hash_result"));
