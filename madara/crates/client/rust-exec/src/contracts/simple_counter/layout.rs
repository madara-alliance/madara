//! Storage layout for SimpleCounter contract.
//!
//! This contract has a single storage variable: `counter: u128`

use once_cell::sync::Lazy;

use crate::storage::storage_key_for_variable;
use crate::types::StorageKey;

/// Storage key for the `counter` variable.
///
/// In Cairo: `counter: u128` becomes `sn_keccak("counter")` as the storage key.
pub static COUNTER_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("counter"));
