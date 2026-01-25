//! Storage layout for SimpleCounter contract.
//!
//! This contract has a single storage variable: `x: felt252`

use once_cell::sync::Lazy;

use crate::storage::storage_key_for_variable;
use crate::types::StorageKey;

/// Storage key for the `x` variable.
///
/// In Cairo: `x: felt252` becomes `sn_keccak("x")` as the storage key.
pub static X_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("x"));
