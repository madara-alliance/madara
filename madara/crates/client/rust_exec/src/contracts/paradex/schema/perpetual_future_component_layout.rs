// @Paradex contract schema
// Contract: perpetual_future_component

use crate::storage::{storage_key_for_map2_poseidon, storage_key_for_map_poseidon, storage_key_for_variable};
use crate::types::StorageKey;
use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

pub static PARACLEAR_PERPETUAL_ASSET_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_perpetual_asset"));
pub fn Paraclear_perpetual_asset_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_perpetual_asset", k)
}

pub static PARACLEAR_PERPETUAL_ASSET_MIN_SIZE_INCREMENT_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_perpetual_asset_min_size_increment"));
pub fn Paraclear_perpetual_asset_min_size_increment_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_perpetual_asset_min_size_increment", k)
}

pub static PARACLEAR_PERPETUAL_ASSET_BALANCE_TAIL_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_perpetual_asset_balance_tail"));
pub fn Paraclear_perpetual_asset_balance_tail_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_perpetual_asset_balance_tail", k)
}

pub static PARACLEAR_PERPETUAL_ASSET_BALANCE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_perpetual_asset_balance"));
pub fn Paraclear_perpetual_asset_balance_key2(k1: Felt, k2: Felt) -> StorageKey {
    storage_key_for_map2_poseidon("Paraclear_perpetual_asset_balance", k1, k2)
}

pub static PARACLEAR_TOTAL_PERPETUAL_ASSET_REALIZED_PNL_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_total_perpetual_asset_realized_pnl"));
pub fn Paraclear_total_perpetual_asset_realized_pnl_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_total_perpetual_asset_realized_pnl", k)
}

pub static PARACLEAR_TOTAL_PERPETUAL_ASSET_REALIZED_FUNDING_PNL_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_total_perpetual_asset_realized_funding_pnl"));
pub fn Paraclear_total_perpetual_asset_realized_funding_pnl_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_total_perpetual_asset_realized_funding_pnl", k)
}

pub static INVARIANTS_PERPETUAL_ASSET_INFO_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("invariants_perpetual_asset_info"));
pub fn invariants_perpetual_asset_info_key2(k1: Felt, k2: Felt) -> StorageKey {
    storage_key_for_map2_poseidon("invariants_perpetual_asset_info", k1, k2)
}

pub static PERPETUAL_FUTURES_MMF_FACTOR_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("perpetual_futures_mmf_factor"));

pub static PERPETUAL_FUTURE_MARKET_FEE_CONFIG_V2_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("perpetual_future_market_fee_config_v2"));
pub fn perpetual_future_market_fee_config_v2_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("perpetual_future_market_fee_config_v2", k)
}

pub static PERPETUAL_FUTURE_MAX_FEE_RATE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("perpetual_future_max_fee_rate"));
pub fn perpetual_future_max_fee_rate_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("perpetual_future_max_fee_rate", k)
}
