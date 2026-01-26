//! Storage layout for HeavyTradeSimulator contract.
//!
//! This module defines storage key computation for the contract's storage mappings.
//!
//! # Storage Key Computation (from starknet_api)
//!
//! ```
//! fn get_storage_var_address(var_name: &str, args: &[Felt]) -> StorageKey {
//!     let base = starknet_keccak(var_name.as_bytes());
//!     let key = args.iter().fold(base, |res, arg| Pedersen::hash(&res, arg));
//!     key % L2_ADDRESS_UPPER_BOUND  // L2_ADDRESS_UPPER_BOUND = 2^251 - 256
//! }
//! ```

use once_cell::sync::Lazy;
use starknet_types_core::felt::{Felt, NonZeroFelt};
use starknet_types_core::hash::{Pedersen, StarkHash};

use crate::storage::sn_keccak;
use crate::types::{ContractAddress, StorageKey};

/// L2_ADDRESS_UPPER_BOUND = 2^251 - 256 (from starknet_api::core)
/// This is used to constrain storage keys to valid Patricia key range.
static L2_ADDRESS_UPPER_BOUND: Lazy<NonZeroFelt> = Lazy::new(|| {
    // 2^251
    let two_pow_251 = Felt::from_hex_unchecked("0x800000000000000000000000000000000000000000000000000000000000000");
    // 2^251 - 256
    let bound = two_pow_251 - Felt::from(256u64);
    NonZeroFelt::try_from(bound).expect("L2_ADDRESS_UPPER_BOUND is non-zero")
});

// Storage variable bases (computed as sn_keccak of variable names)

/// account_states: Map<ContractAddress, AccountState>
pub static ACCOUNT_STATES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_states"));

/// account_asset_balances: Map<(ContractAddress, felt252), AssetBalance>
pub static ACCOUNT_ASSET_BALANCES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_asset_balances"));

/// account_referrers: Map<ContractAddress, ContractAddress>
pub static ACCOUNT_REFERRERS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_referrers"));

/// account_fee_overrides: Map<ContractAddress, FeeConfig>
pub static ACCOUNT_FEE_OVERRIDES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"account_fee_overrides"));

/// market_states: Map<felt252, MarketState>
pub static MARKET_STATES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"market_states"));

/// market_fee_configs: Map<felt252, FeeConfig>
pub static MARKET_FEE_CONFIGS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"market_fee_configs"));

/// market_oracles: Map<felt252, ContractAddress>
pub static MARKET_ORACLES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"market_oracles"));

/// perpetual_positions: Map<(ContractAddress, felt252), i128>
pub static PERPETUAL_POSITIONS_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"perpetual_positions"));

/// perpetual_funding_paid: Map<(ContractAddress, felt252), i128>
pub static PERPETUAL_FUNDING_PAID_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"perpetual_funding_paid"));

/// perpetual_realized_pnl: Map<(ContractAddress, felt252), i128>
pub static PERPETUAL_REALIZED_PNL_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"perpetual_realized_pnl"));

/// fee_collector: ContractAddress
pub static FEE_COLLECTOR_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"fee_collector").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

/// insurance_fund: ContractAddress
pub static INSURANCE_FUND_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"insurance_fund").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

/// fee_share_account: ContractAddress
pub static FEE_SHARE_ACCOUNT_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"fee_share_account").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

/// global_fee_config: FeeConfig
pub static GLOBAL_FEE_CONFIG_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"global_fee_config").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

/// total_trades_settled: u128
pub static TOTAL_TRADES_SETTLED_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"total_trades_settled").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

/// total_volume: u128
pub static TOTAL_VOLUME_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"total_volume").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

/// total_hashes_computed: u128
pub static TOTAL_HASHES_COMPUTED_KEY: Lazy<StorageKey> = Lazy::new(|| {
    StorageKey(sn_keccak(b"total_hashes_computed").mod_floor(&L2_ADDRESS_UPPER_BOUND))
});

// ============================================================================
// Storage Key Computation Functions
// Based on starknet_api::abi::abi_utils::get_storage_var_address
// ============================================================================

/// Compute storage key for a simple mapping: Map<K, V>
/// key = pedersen(base, k) % L2_ADDRESS_UPPER_BOUND
fn compute_map_key(base: Felt, key: Felt) -> StorageKey {
    let hash = Pedersen::hash(&base, &key);
    StorageKey(hash.mod_floor(&L2_ADDRESS_UPPER_BOUND))
}

/// Compute storage key for a tuple mapping: Map<(K1, K2), V>
/// Uses nested fold style: hash(hash(base, k1), k2) % L2_ADDRESS_UPPER_BOUND
/// This matches starknet_api's args.iter().fold(base, |res, arg| Pedersen::hash(&res, arg))
fn compute_tuple_map_key(base: Felt, key1: Felt, key2: Felt) -> StorageKey {
    let intermediate = Pedersen::hash(&base, &key1);
    let final_hash = Pedersen::hash(&intermediate, &key2);
    StorageKey(final_hash.mod_floor(&L2_ADDRESS_UPPER_BOUND))
}

// ============================================================================
// Account State Keys
// ============================================================================

/// Get storage key for account_states[account].
/// Returns the base key; struct fields are at consecutive slots.
pub fn account_state_key(account: ContractAddress) -> StorageKey {
    compute_map_key(*ACCOUNT_STATES_BASE, account.0)
}

/// Get storage key for account_asset_balances[(account, asset)].
pub fn account_asset_balance_key(account: ContractAddress, asset: Felt) -> StorageKey {
    compute_tuple_map_key(*ACCOUNT_ASSET_BALANCES_BASE, account.0, asset)
}

/// Get storage key for account_referrers[account].
pub fn account_referrer_key(account: ContractAddress) -> StorageKey {
    compute_map_key(*ACCOUNT_REFERRERS_BASE, account.0)
}

/// Get storage key for account_fee_overrides[account].
pub fn account_fee_override_key(account: ContractAddress) -> StorageKey {
    compute_map_key(*ACCOUNT_FEE_OVERRIDES_BASE, account.0)
}

// ============================================================================
// Market State Keys
// ============================================================================

/// Get storage key for market_states[market_id].
pub fn market_state_key(market_id: Felt) -> StorageKey {
    compute_map_key(*MARKET_STATES_BASE, market_id)
}

/// Get storage key for market_fee_configs[market_id].
pub fn market_fee_config_key(market_id: Felt) -> StorageKey {
    compute_map_key(*MARKET_FEE_CONFIGS_BASE, market_id)
}

/// Get storage key for market_oracles[market_id].
pub fn market_oracle_key(market_id: Felt) -> StorageKey {
    compute_map_key(*MARKET_ORACLES_BASE, market_id)
}

// ============================================================================
// Perpetual Position Keys
// ============================================================================

/// Get storage key for perpetual_positions[(account, market_id)].
pub fn perpetual_position_key(account: ContractAddress, market_id: Felt) -> StorageKey {
    compute_tuple_map_key(*PERPETUAL_POSITIONS_BASE, account.0, market_id)
}

/// Get storage key for perpetual_funding_paid[(account, market_id)].
pub fn perpetual_funding_paid_key(account: ContractAddress, market_id: Felt) -> StorageKey {
    compute_tuple_map_key(*PERPETUAL_FUNDING_PAID_BASE, account.0, market_id)
}

/// Get storage key for perpetual_realized_pnl[(account, market_id)].
pub fn perpetual_realized_pnl_key(account: ContractAddress, market_id: Felt) -> StorageKey {
    compute_tuple_map_key(*PERPETUAL_REALIZED_PNL_BASE, account.0, market_id)
}

// ============================================================================
// Helper: Read struct from consecutive storage slots
// ============================================================================

/// Get keys for reading a struct stored at consecutive slots.
pub fn struct_keys(base_key: StorageKey, num_fields: usize) -> Vec<StorageKey> {
    (0..num_fields)
        .map(|i| StorageKey(base_key.0 + Felt::from(i as u64)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_base_addresses() {
        println!("\n=== HeavyTradeSimulator Storage Base Addresses ===");
        println!("account_states:          {:#x}", *ACCOUNT_STATES_BASE);
        println!("account_asset_balances:  {:#x}", *ACCOUNT_ASSET_BALANCES_BASE);
        println!("account_referrers:       {:#x}", *ACCOUNT_REFERRERS_BASE);
        println!("market_states:           {:#x}", *MARKET_STATES_BASE);
        println!("market_fee_configs:      {:#x}", *MARKET_FEE_CONFIGS_BASE);
        println!("perpetual_positions:     {:#x}", *PERPETUAL_POSITIONS_BASE);
        println!("total_hashes_computed:   {:#x}", TOTAL_HASHES_COMPUTED_KEY.0);
        println!("total_trades_settled:    {:#x}", TOTAL_TRADES_SETTLED_KEY.0);
        println!("total_volume:            {:#x}", TOTAL_VOLUME_KEY.0);
        println!("=== End Base Addresses ===\n");
    }

    #[test]
    fn test_account_state_key() {
        let account = ContractAddress(Felt::from(0x1234u64));
        let key = account_state_key(account);
        assert_ne!(key.0, Felt::ZERO);
    }

    #[test]
    fn test_perpetual_position_key() {
        let account = ContractAddress(Felt::from(0x1234u64));
        let market = Felt::from(0xABCDu64);
        let key = perpetual_position_key(account, market);
        assert_ne!(key.0, Felt::ZERO);
    }
}
