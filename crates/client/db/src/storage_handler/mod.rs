use std::fmt::Display;

use async_trait::async_trait;
use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use starknet_api::core::{ClassHash, ContractAddress};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_types_core::felt::Felt;

use self::history::HistoryError;
use crate::mapping_db::MappingDbError;

pub mod benchmark;
pub mod block_state_diff;
pub mod class_trie;
pub mod codec;
pub mod contract_class_data;
pub mod contract_class_hashes;
pub(crate) mod contract_data;
pub(crate) mod contract_storage;
pub mod contract_storage_trie;
pub mod contract_trie;
pub mod history;
pub mod primitives;
pub mod query;

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = "0xcontract".as_bytes();
    pub const CLASS: &[u8] = "0xclass".as_bytes();
    pub const TRANSACTION: &[u8] = "0xtransaction".as_bytes();
    pub const EVENT: &[u8] = "0xevent".as_bytes();
}

#[derive(thiserror::Error, Debug)]
pub enum DeoxysStorageError {
    #[error("Failed to initialize trie for {0}")]
    TrieInitError(TrieType),
    #[error("Failed to compute trie root for {0}")]
    TrieRootError(TrieType),
    #[error("Failed to merge transactional state back into {0}")]
    TrieMergeError(TrieType),
    #[error("Failed to retrieve latest id for {0}")]
    TrieIdError(TrieType),
    #[error("Failed to retrieve storage view for {0}")]
    StoraveViewError(StorageType),
    #[error("Failed to insert data into {0}")]
    StorageInsertionError(StorageType),
    #[error("Failed to retrive data from {0}")]
    StorageRetrievalError(StorageType),
    #[error("Failed to commit to {0}")]
    StorageCommitError(StorageType),
    #[error("Failed to encode {0}")]
    StorageEncodeError(StorageType),
    #[error("Failed to decode {0}")]
    StorageDecodeError(StorageType),
    #[error("Failed to serialize/deserialize")]
    StorageSerdeError,
    #[error("Failed to revert {0} to block {1}")]
    StorageRevertError(StorageType, u64),
    #[error("{0:#}")]
    StorageHistoryError(#[from] HistoryError),
    #[error("{0:#}")]
    MappingDbError(#[from] MappingDbError),
    #[error("Invalid block number")]
    InvalidBlockNumber,
    #[error("Invalid nonce")]
    InvalidNonce,
}

impl From<bincode::Error> for DeoxysStorageError {
    fn from(_: bincode::Error) -> Self {
        DeoxysStorageError::StorageSerdeError
    }
}

#[derive(Debug)]
pub enum TrieType {
    Contract,
    ContractStorage,
    Class,
}

#[derive(Debug)]
pub enum StorageType {
    Contract,
    ContractStorage,
    ContractClassData,
    ContractData,
    ContractAbi,
    ContractClassHashes,
    Class,
    BlockNumber,
    BlockHash,
    BlockStateDiff,
}

impl Display for TrieType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let trie_type = match self {
            TrieType::Contract => "contract trie",
            TrieType::ContractStorage => "contract storage trie",
            TrieType::Class => "class trie",
        };

        write!(f, "{trie_type}")
    }
}

impl Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let storage_type = match self {
            StorageType::Contract => "contract",
            StorageType::ContractStorage => "contract storage",
            StorageType::Class => "class storage",
            StorageType::ContractClassData => "class definition storage",
            StorageType::ContractAbi => "class abi storage",
            StorageType::BlockNumber => "block number storage",
            StorageType::BlockHash => "block hash storage",
            StorageType::BlockStateDiff => "block state diff storage",
            StorageType::ContractClassHashes => "contract class hashes storage",
            StorageType::ContractData => "contract class data storage",
        };

        write!(f, "{storage_type}")
    }
}

/// An immutable view on a backend storage interface.
///
/// > Multiple immutable views can exist at once for a same storage type.
/// > You cannot have an immutable view and a mutable view on storage at the same time.
///
/// Use this to query data from the backend database in a type-safe way.
pub trait StorageView {
    type KEY;
    type VALUE;

    /// Retrieves data from storage for the given key
    ///
    /// * `key`: identifier used to retrieve the data.
    fn get(&self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError>;

    /// Checks if a value is stored in the backend database for the given key.
    ///
    /// * `key`: identifier use to check for data existence.
    fn contains(&self, key: &Self::KEY) -> Result<bool, DeoxysStorageError>;
}

/// A mutable view on a backend storage interface.
///
/// > Note that a single mutable view can exist at once for a same storage type.
///
/// Use this to write data to the backend database in a type-safe way.
pub trait StorageViewMut {
    type KEY;
    type VALUE;

    /// Insert data into storage.
    ///
    /// * `key`: identifier used to inser data.
    /// * `value`: encodable data to save to the database.
    fn insert(&self, key: Self::KEY, value: Self::VALUE) -> Result<(), DeoxysStorageError>;

    /// Applies all changes up to this point.
    ///
    /// * `block_number`: point in the chain at which to apply the new changes. Must be incremental
    fn commit(self, block_number: u64) -> Result<(), DeoxysStorageError>;
}

/// A mutable view on a backend storage interface, marking it as revertible in the chain.
///
/// This is used to mark data that might be modified from one block to the next, such as contract
/// storage.
#[async_trait]
pub trait StorageViewRevetible: StorageViewMut {
    /// Reverts to a previous state in the chain.
    ///
    /// * `block_number`: point in the chain to revert to.
    async fn revert_to(&self, block_number: u64) -> Result<(), DeoxysStorageError>;
}

fn conv_contract_identifier(identifier: &ContractAddress) -> &[u8] {
    identifier.0.key().bytes()
}

fn conv_contract_key(key: &ContractAddress) -> BitVec<u8, Msb0> {
    let bytes = key.0.key().bytes();
    bytes.as_bits()[5..].to_owned()
}

fn conv_contract_storage_key(key: &StorageKey) -> BitVec<u8, Msb0> {
    let bytes = key.0.key().bytes();
    bytes.as_bits()[5..].to_owned()
}

fn conv_contract_value(value: StarkFelt) -> Felt {
    Felt::from_bytes_be_slice(value.bytes())
}

fn conv_class_key(key: &ClassHash) -> BitVec<u8, Msb0> {
    let bytes = key.0.bytes();
    bytes.as_bits()[5..].to_owned()
}
