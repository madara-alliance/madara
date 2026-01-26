//! ERC20 storage layout.
//!
//! This module defines the storage layout for ERC20 tokens (ETH/STRK fee tokens).
//!
//! # Storage Key Computation
//!
//! Based on starknet_api::abi::abi_utils::get_storage_var_address:
//! ```
//! fn get_storage_var_address(var_name: &str, args: &[Felt]) -> StorageKey {
//!     let base = starknet_keccak(var_name.as_bytes());
//!     let key = args.iter().fold(base, |res, arg| Pedersen::hash(&res, arg));
//!     key % L2_ADDRESS_UPPER_BOUND  // L2_ADDRESS_UPPER_BOUND = 2^251 - 256
//! }
//! ```
//!
//! For ERC20 balances: `get_storage_var_address("ERC20_balances", &[address])`

use once_cell::sync::Lazy;
use starknet_types_core::felt::{Felt, NonZeroFelt};
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

use crate::storage::sn_keccak;
use crate::types::{ContractAddress, StorageKey};

/// L2_ADDRESS_UPPER_BOUND = 2^251 - 256 (from starknet_api::core)
/// This is used to constrain storage keys to valid Patricia key range.
pub static L2_ADDRESS_UPPER_BOUND: Lazy<NonZeroFelt> = Lazy::new(|| {
    // 2^251
    let two_pow_251 = Felt::from_hex_unchecked("0x800000000000000000000000000000000000000000000000000000000000000");
    // 2^251 - 256
    let bound = two_pow_251 - Felt::from(256u64);
    NonZeroFelt::try_from(bound).expect("L2_ADDRESS_UPPER_BOUND should be non-zero")
});

// ============================================================================
// Storage Variable Bases
// ============================================================================

/// Storage key for ERC20_balances variable base.
pub static ERC20_BALANCES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"ERC20_balances"));

/// Storage key for ERC20_allowances variable base.
pub static ERC20_ALLOWANCES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"ERC20_allowances"));

/// Storage key for ERC20_total_supply variable.
pub static ERC20_TOTAL_SUPPLY_KEY: Lazy<StorageKey> = Lazy::new(|| StorageKey(sn_keccak(b"ERC20_total_supply")));

/// Storage key for ERC20_name variable.
pub static ERC20_NAME_KEY: Lazy<StorageKey> = Lazy::new(|| StorageKey(sn_keccak(b"ERC20_name")));

/// Storage key for ERC20_symbol variable.
pub static ERC20_SYMBOL_KEY: Lazy<StorageKey> = Lazy::new(|| StorageKey(sn_keccak(b"ERC20_symbol")));

/// Storage key for ERC20_decimals variable.
pub static ERC20_DECIMALS_KEY: Lazy<StorageKey> = Lazy::new(|| StorageKey(sn_keccak(b"ERC20_decimals")));

// ============================================================================
// Storage Key Computation (matches starknet_api::abi::abi_utils)
// ============================================================================

/// Compute storage key using the exact algorithm from starknet_api.
///
/// This matches `get_storage_var_address(var_name, &[address])`:
/// 1. base = starknet_keccak(var_name)
/// 2. key = Pedersen::hash(&base, &address)  // Note: base first, then address!
/// 3. key = key % L2_ADDRESS_UPPER_BOUND
fn compute_storage_key(base: Felt, address: Felt) -> Felt {
    // Pedersen hash with correct order: (base, address)
    let key = Pedersen::hash(&base, &address);
    // Apply modulo to constrain to valid storage key range
    key.mod_floor(&L2_ADDRESS_UPPER_BOUND)
}

/// Compute the storage key for a balance.
///
/// Matches: `get_storage_var_address("ERC20_balances", &[address])`
pub fn balance_key(address: ContractAddress) -> StorageKey {
    StorageKey(compute_storage_key(*ERC20_BALANCES_BASE, address.0))
}

/// Compute the storage key for the high 128 bits of a balance (u256 high part).
pub fn balance_key_high(address: ContractAddress) -> StorageKey {
    StorageKey(compute_storage_key(*ERC20_BALANCES_BASE, address.0) + Felt::ONE)
}

/// Compute the storage key for an allowance.
///
/// Matches: `get_storage_var_address("ERC20_allowances", &[owner, spender])`
pub fn allowance_key(owner: ContractAddress, spender: ContractAddress) -> StorageKey {
    // For nested mapping, fold: hash(hash(base, owner), spender)
    let key1 = Pedersen::hash(&ERC20_ALLOWANCES_BASE, &owner.0);
    let key2 = Pedersen::hash(&key1, &spender.0);
    StorageKey(key2.mod_floor(&L2_ADDRESS_UPPER_BOUND))
}

/// Compute the storage key for the high 128 bits of an allowance.
pub fn allowance_key_high(owner: ContractAddress, spender: ContractAddress) -> StorageKey {
    let key1 = Pedersen::hash(&ERC20_ALLOWANCES_BASE, &owner.0);
    let key2 = Pedersen::hash(&key1, &spender.0);
    StorageKey(key2.mod_floor(&L2_ADDRESS_UPPER_BOUND) + Felt::ONE)
}

// ============================================================================
// Cairo 1.0 Layout (Poseidon-based)
// ============================================================================

/// Common variable names used in Cairo 1.0 ERC20 implementations.
pub const CAIRO1_VARIABLE_NAMES: &[&str] = &[
    "ERC20_balances",
    "balances",
    "ERC20Balances",
    "ERC20_balance",
    "balance",
    "Balances",
    "ERC20Component_balances",
    "token_balances",
];

/// Compute the storage key for a balance using Cairo 1.0 Poseidon hash.
///
/// For Cairo 1.0 `balances: LegacyMap<ContractAddress, u256>`:
/// key = poseidon(sn_keccak(variable_name), address)
pub fn balance_key_poseidon(variable_name: &str, address: ContractAddress) -> StorageKey {
    let base = sn_keccak(variable_name.as_bytes());
    StorageKey(Poseidon::hash_array(&[base, address.0]))
}

/// Compute the storage key for the high 128 bits of a balance (Cairo 1.0 Poseidon).
pub fn balance_key_high_poseidon(variable_name: &str, address: ContractAddress) -> StorageKey {
    let base = sn_keccak(variable_name.as_bytes());
    StorageKey(Poseidon::hash_array(&[base, address.0]) + Felt::ONE)
}

/// Try all known variable names and return the computed keys.
///
/// This is useful for debugging to find which variable name matches Blockifier's keys.
pub fn try_all_balance_keys(address: ContractAddress) -> Vec<BalanceKeyVariant> {
    let mut results = Vec::new();

    // Correct algorithm: pedersen(base, addr) with modulo (matches starknet_api)
    results.push(BalanceKeyVariant {
        name: "ERC20_balances",
        hash_type: "pedersen(base,addr) % L2_BOUND",
        key_low: balance_key(address),
        key_high: balance_key_high(address),
    });

    // Try other variable names with correct algorithm
    for name in &["balances", "ERC20_balance", "balance", "Balances"] {
        let base = sn_keccak(name.as_bytes());
        let key = Pedersen::hash(&base, &address.0).mod_floor(&L2_ADDRESS_UPPER_BOUND);
        results.push(BalanceKeyVariant {
            name,
            hash_type: "pedersen(base,addr) % L2_BOUND",
            key_low: StorageKey(key),
            key_high: StorageKey(key + Felt::ONE),
        });
    }

    // Cairo 1.0 Poseidon-based (less likely for fee tokens, but try anyway)
    for name in CAIRO1_VARIABLE_NAMES {
        results.push(BalanceKeyVariant {
            name,
            hash_type: "poseidon(base, addr)",
            key_low: balance_key_poseidon(name, address),
            key_high: balance_key_high_poseidon(name, address),
        });
    }

    results
}

/// A balance key variant with metadata.
#[derive(Debug)]
pub struct BalanceKeyVariant {
    pub name: &'static str,
    pub hash_type: &'static str,
    pub key_low: StorageKey,
    pub key_high: StorageKey,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_key_computation() {
        let address = ContractAddress(Felt::from(0x1234u64));
        let key = balance_key(address);
        // Just verify it produces a valid key
        assert_ne!(key.0, Felt::ZERO);
    }

    #[test]
    fn test_different_addresses_different_keys() {
        let addr1 = ContractAddress(Felt::from(1u64));
        let addr2 = ContractAddress(Felt::from(2u64));

        let key1 = balance_key(addr1);
        let key2 = balance_key(addr2);

        assert_ne!(key1.0, key2.0);
    }
}
