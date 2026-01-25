//! State reader trait and related types.
//!
//! This module defines the interface for reading blockchain state,
//! allowing the execution context to read from different state sources.

use starknet_types_core::felt::Felt;
use thiserror::Error;

use crate::types::{ContractAddress, Nonce, StorageKey};

/// Errors that can occur when reading state.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("Failed to read storage at contract {contract:?} key {key:?}: {reason}")]
    StorageReadFailed { contract: ContractAddress, key: StorageKey, reason: String },

    #[error("Failed to read nonce for contract {contract:?}: {reason}")]
    NonceReadFailed { contract: ContractAddress, reason: String },

    #[error("Failed to read class hash for contract {contract:?}: {reason}")]
    ClassHashReadFailed { contract: ContractAddress, reason: String },

    #[error("Invalid contract address: {0}")]
    InvalidAddress(String),

    #[error("Invalid storage key: {0}")]
    InvalidStorageKey(String),

    #[error("Backend error: {0}")]
    BackendError(String),
}

/// Trait for reading blockchain state.
///
/// This abstracts over different state sources:
/// - Direct database access
/// - CachedState wrapper
/// - Mock state for testing
pub trait StateReader {
    /// Read a storage value at the given contract and key.
    ///
    /// Returns `Felt::ZERO` if the key has never been written.
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> Result<Felt, StateError>;

    /// Read the nonce for a contract.
    ///
    /// Returns `Nonce(0)` if the contract has no nonce set.
    fn get_nonce_at(&self, contract_address: ContractAddress) -> Result<Nonce, StateError>;

    /// Read the class hash for a contract.
    ///
    /// Returns `None` if the contract is not deployed.
    fn get_class_hash_at(&self, contract_address: ContractAddress) -> Result<Option<Felt>, StateError>;
}

/// A mock state reader for testing.
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::HashMap;

    #[derive(Debug, Default)]
    pub struct MockStateReader {
        pub storage: HashMap<(ContractAddress, StorageKey), Felt>,
        pub nonces: HashMap<ContractAddress, Nonce>,
        pub class_hashes: HashMap<ContractAddress, Felt>,
    }

    impl MockStateReader {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn set_storage(&mut self, contract: ContractAddress, key: StorageKey, value: Felt) {
            self.storage.insert((contract, key), value);
        }

        pub fn set_nonce(&mut self, contract: ContractAddress, nonce: Nonce) {
            self.nonces.insert(contract, nonce);
        }

        pub fn set_class_hash(&mut self, contract: ContractAddress, class_hash: Felt) {
            self.class_hashes.insert(contract, class_hash);
        }
    }

    impl StateReader for MockStateReader {
        fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> Result<Felt, StateError> {
            Ok(self.storage.get(&(contract_address, key)).copied().unwrap_or(Felt::ZERO))
        }

        fn get_nonce_at(&self, contract_address: ContractAddress) -> Result<Nonce, StateError> {
            Ok(self.nonces.get(&contract_address).copied().unwrap_or(Nonce(Felt::ZERO)))
        }

        fn get_class_hash_at(&self, contract_address: ContractAddress) -> Result<Option<Felt>, StateError> {
            Ok(self.class_hashes.get(&contract_address).copied())
        }
    }
}
