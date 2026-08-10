// @Paradex contract schema
// Contract: ParaclearOracle

use crate::storage::{storage_key_for_map2_poseidon, storage_key_for_map_poseidon, storage_key_for_variable};
use crate::types::StorageKey;
use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

pub static ACCESSCONTROL_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("accesscontrol"));

pub static SRC5_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("src5"));

pub static UPGRADEABLE_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("upgradeable"));

pub static LATEST_TICK_DATA_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("latest_tick_data"));
pub fn latest_tick_data_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("latest_tick_data", k)
}

pub static LATEST_UPDATED_TIMESTAMP_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("latest_updated_timestamp"));

pub static FUNDING_INDEX_DATA_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("funding_index_data"));
pub fn funding_index_data_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("funding_index_data", k)
}

pub static LATEST_SNAPSHOT_ID_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("latest_snapshot_id"));
