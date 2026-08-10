// @Paradex contract schema
// Contract: Paraclear

use crate::core::storage::{
    storage_key_for_map, storage_key_for_map2_poseidon, storage_key_for_map_poseidon, storage_key_for_variable,
};
use crate::core::types::StorageKey;
use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

pub static ACCESSCONTROL_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("accesscontrol"));

pub static SRC5_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("src5"));

pub static UPGRADEABLE_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("upgradeable"));

pub static PERPETUAL_FUTURE_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("perpetual_future"));

pub static PERPETUAL_OPTION_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("perpetual_option"));

pub static PERPETUAL_ASSET_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("perpetual_asset"));

pub static TOKEN_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("token"));

pub static ACCOUNT_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("account"));

pub static GLOBAL_CONFIGURATION_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("global_configuration"));

pub static PARACLEAR_SETTLEMENT_TOKEN_ASSET_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_settlement_token_asset"));

pub static PARACLEAR_LIQUIDATION_FEE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_liquidation_fee"));

pub static PARACLEAR_LIQUIDATION_INSURANCE_FUND_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_liquidation_insurance_fund"));

pub static PARACLEAR_ORACLE_CONTRACT_ADDRESS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_oracle_contract_address"));

pub static PARACLEAR_FEE_ACCOUNT_ADDRESS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_fee_account_address"));

pub static PARACLEAR_BRIDGE_CONTRACT_ADDRESS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_bridge_contract_address"));

pub static PARACLEAR_MARKET_DELEGATE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_market_delegate"));
pub fn Paraclear_market_delegate_key(k: Felt) -> StorageKey {
    storage_key_for_map("Paraclear_market_delegate", k)
}

pub static PARACLEAR_FEE_SHARE_ACCOUNT_ADDRESS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_fee_share_account_address"));

pub static PARACLEAR_FEE_SHARE_PERCENTAGE_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("Paraclear_fee_share_percentage"));

pub static WHITELISTED_EXECUTOR_ACCOUNTS_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("whitelisted_executor_accounts"));

pub static PENDING_TRANSFER_COUNT_BY_EXECUTOR_BASE: Lazy<StorageKey> =
    Lazy::new(|| storage_key_for_variable("pending_transfer_count_by_executor"));
pub fn pending_transfer_count_by_executor_key(k: Felt) -> StorageKey {
    storage_key_for_map_poseidon("pending_transfer_count_by_executor", k)
}

pub static PENDING_TRANSFERS_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("pending_transfers"));
pub fn pending_transfers_key2(k1: Felt, k2: Felt) -> StorageKey {
    storage_key_for_map2_poseidon("pending_transfers", k1, k2)
}

pub static ASSETS_MANAGER_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable("assets_manager"));
