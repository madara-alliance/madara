//! Storage layout for MathBenchmark contract.

use once_cell::sync::Lazy;

use crate::storage::storage_key_for_variable;
use crate::types::StorageKey;

/// Storage key for `last_result_u128: u128`
pub static LAST_RESULT_U128_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("last_result_u128"));

/// Storage key for `last_result_i128: felt252`
pub static LAST_RESULT_I128_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("last_result_i128"));

/// Storage key for `last_result_felt: felt252`
pub static LAST_RESULT_FELT_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("last_result_felt"));

/// Storage key for `event_count: u128`
pub static EVENT_COUNT_KEY: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("event_count"));
