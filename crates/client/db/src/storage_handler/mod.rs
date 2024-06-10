use std::fmt::Display;

use async_trait::async_trait;
use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use starknet_api::core::{ClassHash, ContractAddress};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_types_core::felt::Felt;

use self::block_state_diff::BlockStateDiffView;
use self::class_trie::{ClassTrieView, ClassTrieViewMut};
use self::contract_class_data::{ContractClassDataView, ContractClassDataViewMut};
use self::contract_class_hashes::{ContractClassHashesView, ContractClassHashesViewMut};
use self::contract_data::{ContractClassView, ContractClassViewMut, ContractNoncesView, ContractNoncesViewMut};
use self::contract_storage::{ContractStorageView, ContractStorageViewMut};
use self::contract_storage_trie::{ContractStorageTrieView, ContractStorageTrieViewMut};
use self::contract_trie::{ContractTrieView, ContractTrieViewMut};
use self::history::HistoryError;
use crate::mapping_db::MappingDbError;
use crate::DeoxysBackend;

pub mod benchmark;
pub mod block_state_diff;
mod class_trie;
pub mod codec;
mod contract_class_data;
mod contract_class_hashes;
pub(crate) mod contract_data;
pub(crate) mod contract_storage;
mod contract_storage_trie;
mod contract_trie;
mod history;
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
    #[error("failed to initialize trie for {0}")]
    TrieInitError(TrieType),
    #[error("failed to compute trie root for {0}")]
    TrieRootError(TrieType),
    #[error("failed to merge transactional state back into {0}")]
    TrieMergeError(TrieType),
    #[error("failed to retrieve latest id for {0}")]
    TrieIdError(TrieType),
    #[error("failed to retrieve storage view for {0}")]
    StoraveViewError(StorageType),
    #[error("failed to insert data into {0}")]
    StorageInsertionError(StorageType),
    #[error("failed to retrive data from {0}")]
    StorageRetrievalError(StorageType),
    #[error("failed to commit to {0}")]
    StorageCommitError(StorageType),
    #[error("failed to encode {0}")]
    StorageEncodeError(StorageType),
    #[error("failed to decode {0}")]
    StorageDecodeError(StorageType),
    #[error("failed to serialize/deserialize")]
    StorageSerdeError,
    #[error("failed to revert {0} to block {1}")]
    StorageRevertError(StorageType, u64),
    #[error("failed to parse history")]
    StorageHistoryError(#[from] HistoryError),
    #[error("mapping db error")]
    MappingDbError(#[from] MappingDbError),
    #[error("invalid block number")]
    InvalidBlockNumber,
    #[error("invalid nonce")]
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

pub fn contract_trie_mut<'a>() -> ContractTrieViewMut<'a> {
    ContractTrieViewMut(DeoxysBackend::bonsai_contract().write().unwrap())
}

pub fn contract_trie<'a>() -> ContractTrieView<'a> {
    ContractTrieView(DeoxysBackend::bonsai_contract().read().unwrap())
}

pub fn contract_storage_trie_mut<'a>() -> ContractStorageTrieViewMut<'a> {
    ContractStorageTrieViewMut(DeoxysBackend::bonsai_storage().write().unwrap())
}

pub fn contract_storage_trie<'a>() -> ContractStorageTrieView<'a> {
    ContractStorageTrieView(DeoxysBackend::bonsai_storage().read().unwrap())
}

pub fn contract_storage_mut() -> ContractStorageViewMut {
    ContractStorageViewMut::new()
}

pub fn contract_storage() -> ContractStorageView {
    ContractStorageView::new()
}

pub fn class_trie_mut<'a>() -> ClassTrieViewMut<'a> {
    ClassTrieViewMut(DeoxysBackend::bonsai_class().write().unwrap())
}

pub fn class_trie<'a>() -> ClassTrieView<'a> {
    ClassTrieView(DeoxysBackend::bonsai_class().read().unwrap())
}

pub fn contract_class_data_mut() -> ContractClassDataViewMut {
    ContractClassDataViewMut::default()
}

pub fn contract_class_data() -> ContractClassDataView {
    ContractClassDataView
}

pub fn contract_class_hashes_mut() -> ContractClassHashesViewMut {
    ContractClassHashesViewMut::default()
}

pub fn contract_class_hashes() -> ContractClassHashesView {
    ContractClassHashesView
}

pub fn contract_class_hash() -> ContractClassView {
    ContractClassView::new()
}

pub fn contract_class_hash_mut() -> ContractClassViewMut {
    ContractClassViewMut::new()
}

pub fn contract_nonces() -> ContractNoncesView {
    ContractNoncesView::new()
}

pub fn contract_nonces_mut() -> ContractNoncesViewMut {
    ContractNoncesViewMut::new()
}

pub fn block_state_diff() -> BlockStateDiffView {
    BlockStateDiffView
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
