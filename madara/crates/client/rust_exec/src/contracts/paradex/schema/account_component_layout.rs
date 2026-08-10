// @Paradex contract schema
// Contract: account_component

use crate::core::storage::{
    storage_key_for_map, storage_key_for_map2_poseidon, storage_key_for_map_poseidon, storage_key_for_variable,
};
use crate::core::types::StorageKey;
use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

pub static PARACLEAR_ACCOUNT_TAIL_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_account_tail"));

pub static PARACLEAR_ACCOUNT_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("Paraclear_account"));
pub fn Paraclear_account_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_account", k)
}

pub static PARACLEAR_ACCOUNT_REFERRAL_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_account_referral"));
pub fn Paraclear_account_referral_key(k: Felt) -> StorageKey {
    storage_key_for_map("Paraclear_account_referral", k)
}

pub static PARACLEAR_GLOBAL_FEE_RATE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_global_fee_rate"));

pub static PARACLEAR_ACCOUNT_FEE_RATE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_account_fee_rate"));
pub fn Paraclear_account_fee_rate_key(k: Felt) -> StorageKey {
    storage_key_for_map("Paraclear_account_fee_rate", k)
}

pub static PARACLEAR_GLOBAL_FEE_RATE_OPTIONS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_global_fee_rate_options"));

pub static PARACLEAR_ACCOUNT_FEE_RATE_OPTIONS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_account_fee_rate_options"));
pub fn Paraclear_account_fee_rate_options_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_account_fee_rate_options", k)
}

pub static PARACLEAR_GLOBAL_FEE_RATE_SPOT_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_global_fee_rate_spot"));

pub static PARACLEAR_ACCOUNT_FEE_RATE_SPOT_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_account_fee_rate_spot"));
pub fn Paraclear_account_fee_rate_spot_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("Paraclear_account_fee_rate_spot", k)
}

pub static ACCOUNT_MARGIN_METHODOLOGY_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("account_margin_methodology"));
pub fn account_margin_methodology_key(k: Felt) -> StorageKey {
    storage_key_for_map("account_margin_methodology", k)
}

pub static PARACLEAR_GLOBAL_SETTLEMENT_FEE_RATES_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_global_settlement_fee_rates"));
