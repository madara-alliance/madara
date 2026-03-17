//! Account contract storage layout.
//!
//! This module defines the storage layout for OpenZeppelin-style account contracts.

use once_cell::sync::Lazy;
use starknet_types_core::felt::Felt;

use crate::storage::sn_keccak;
use crate::types::StorageKey;

/// Storage key for the account's public key.
/// This is used for signature verification.
pub static ACCOUNT_PUBLIC_KEY: Lazy<StorageKey> = Lazy::new(|| StorageKey(sn_keccak(b"Account_public_key")));

/// Storage key for OZ's Ownable component owner.
pub static OWNABLE_OWNER: Lazy<StorageKey> = Lazy::new(|| StorageKey(sn_keccak(b"Ownable_owner")));

/// Storage key for SRC5 interface support.
/// Used to check if contract supports certain interfaces.
pub static SRC5_SUPPORTED_INTERFACES_BASE: Lazy<Felt> = Lazy::new(|| sn_keccak(b"SRC5_supported_interfaces"));

/// Account interface ID (SRC-6)
pub const ACCOUNT_INTERFACE_ID: Felt =
    Felt::from_hex_unchecked("0x2ceccef7f994940b3962a6c67e0ba4fcd37df7d131417c604f91e03caecc1cd");

/// Old account interface ID (for backwards compatibility)
pub const OLD_ACCOUNT_INTERFACE_ID: Felt = Felt::from_hex_unchecked("0xa66bd575");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_key_storage_key() {
        // Just verify the key is computed
        let key = *ACCOUNT_PUBLIC_KEY;
        assert_ne!(key.0, Felt::ZERO);
    }
}
