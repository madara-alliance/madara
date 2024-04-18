use std::fmt::Display;
use std::sync::RwLockReadGuard;

use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiDatabase, BonsaiStorage};
use parity_scale_codec::{Decode, Encode};
use sp_core::hexdisplay::AsBytesRef;
use starknet_api::api_core::{ClassHash, ContractAddress};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::StarkHash;
use thiserror::Error;

use self::block_hash::{BlockHashView, BlockHashViewMut};
use self::block_number::{BlockNumberView, BlockNumberViewMut};
use self::class_hash::{ClassHashView, ClassHashViewMut};
use self::class_trie::{ClassTrieView, ClassViewMut};
use self::contract_abi::{ContractAbiView, ContractAbiViewMut};
use self::contract_class::{ContractClassView, ContractClassViewMut};
use self::contract_storage_trie::{ContractStorageView, ContractStorageVueMut};
use self::contract_trie::{ContractView, ContractViewMut};
use crate::DeoxysBackend;

pub mod block_hash;
pub mod block_number;
pub mod class_hash;
pub mod class_trie;
pub mod contract_abi;
pub mod contract_class;
pub mod contract_storage_trie;
pub mod contract_trie;

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = "0xcontract".as_bytes();
    pub const CLASS: &[u8] = "0xclass".as_bytes();
    pub const TRANSACTION: &[u8] = "0xtransaction".as_bytes();
    pub const EVENT: &[u8] = "0xevent".as_bytes();
}

#[derive(Error, Debug)]
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
    #[error("failed to decode {0}")]
    StorageDecodeError(StorageType),
    #[error("failed to revert {0} to block {1}")]
    StorageRevertError(StorageType, u64),
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
    Class,
    ContractClass,
    ContractAbi,
    BlockNumber,
    BlockHash,
    ClassHash,
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
            StorageType::ContractClass => "class definition storage",
            StorageType::ContractAbi => "class abi storage",
            StorageType::BlockNumber => "block number storage",
            StorageType::BlockHash => "block hash storage",
            StorageType::ClassHash => "class hash storage",
        };

        write!(f, "{storage_type}")
    }
}

pub trait StorageView {
    type KEY: Encode + Decode;
    type VALUE: Encode + Decode;

    fn get(self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError>;
    fn contains(self, key: &Self::KEY) -> Result<bool, DeoxysStorageError>;
}

pub trait StorageViewMut {
    type KEY: Encode + Decode;
    type VALUE: Encode + Decode;

    fn insert(&mut self, key: &Self::KEY, value: &Self::VALUE);
    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError>;
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError>;
}

pub fn contract_mut(block_id: BlockId) -> Result<ContractViewMut, DeoxysStorageError> {
    let bonsai_contract = DeoxysBackend::bonsai_contract().read().unwrap();
    let bonsai_id = conv_block_id(&bonsai_contract, block_id, TrieType::Contract)?;
    let config = bonsai_contract.get_config();
    let transactional_storage = bonsai_contract.get_transactional_state(bonsai_id, config);

    match transactional_storage {
        Ok(Some(transactional_storage)) => Ok(ContractViewMut(transactional_storage)),
        _ => Err(DeoxysStorageError::StoraveViewError(StorageType::Contract)),
    }
}

pub fn contract<'a>() -> Result<ContractView<'a>, DeoxysStorageError> {
    Ok(ContractView(
        DeoxysBackend::bonsai_contract()
            .read()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::Contract))?,
    ))
}

pub fn contract_storage_mut(block_id: BlockId) -> Result<ContractStorageVueMut, DeoxysStorageError> {
    let bonsai_storage = DeoxysBackend::bonsai_storage().read().unwrap();
    let bonsai_id = conv_block_id(&bonsai_storage, block_id, TrieType::ContractStorage)?;
    let config = bonsai_storage.get_config();
    let transactional_storage = bonsai_storage.get_transactional_state(bonsai_id, config);

    match transactional_storage {
        Ok(Some(transactional_storage)) => Ok(ContractStorageVueMut(transactional_storage)),
        _ => Err(DeoxysStorageError::StoraveViewError(StorageType::ContractStorage)),
    }
}

pub fn contract_storage<'a>() -> Result<ContractStorageView<'a>, DeoxysStorageError> {
    Ok(ContractStorageView(
        DeoxysBackend::bonsai_storage()
            .read()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ContractStorage))?,
    ))
}

pub fn contract_class_mut<'a>() -> Result<ContractClassViewMut<'a>, DeoxysStorageError> {
    Ok(ContractClassViewMut(
        DeoxysBackend::contract_class()
            .write()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ContractClass))?,
    ))
}

pub fn contract_class<'a>() -> Result<ContractClassView<'a>, DeoxysStorageError> {
    Ok(ContractClassView(
        DeoxysBackend::contract_class()
            .read()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ContractClass))?,
    ))
}

pub fn contract_abi_mut<'a>() -> Result<ContractAbiViewMut<'a>, DeoxysStorageError> {
    Ok(ContractAbiViewMut(
        DeoxysBackend::contract_abi()
            .write()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ContractAbi))?,
    ))
}

pub fn contract_abi<'a>() -> Result<ContractAbiView<'a>, DeoxysStorageError> {
    Ok(ContractAbiView(
        DeoxysBackend::contract_abi()
            .read()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ContractAbi))?,
    ))
}

pub fn class_mut(block_id: BlockId) -> Result<ClassViewMut, DeoxysStorageError> {
    let bonsai_classes = DeoxysBackend::bonsai_class().read().unwrap();
    let bonsai_id = conv_block_id(&bonsai_classes, block_id, TrieType::Class)?;
    let config = bonsai_classes.get_config();
    let transactional_storage = bonsai_classes.get_transactional_state(bonsai_id, config);

    match transactional_storage {
        Ok(Some(transactional_storage)) => Ok(ClassViewMut(transactional_storage)),
        _ => Err(DeoxysStorageError::StoraveViewError(StorageType::Class)),
    }
}

pub fn class<'a>() -> Result<ClassTrieView<'a>, DeoxysStorageError> {
    Ok(ClassTrieView(
        DeoxysBackend::bonsai_class().read().map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::Class))?,
    ))
}

pub fn class_hash_mut<'a>() -> Result<ClassHashViewMut<'a>, DeoxysStorageError> {
    Ok(ClassHashViewMut(
        DeoxysBackend::class_hash()
            .write()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ClassHash))?,
    ))
}

pub fn class_hash<'a>() -> Result<ClassHashView<'a>, DeoxysStorageError> {
    Ok(ClassHashView(
        DeoxysBackend::class_hash().read().map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ClassHash))?,
    ))
}

pub fn block_number_mut<'a>() -> Result<BlockNumberViewMut<'a>, DeoxysStorageError> {
    Ok(BlockNumberViewMut(
        DeoxysBackend::block_number()
            .write()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::BlockNumber))?,
    ))
}

pub fn block_number<'a>() -> Result<BlockNumberView<'a>, DeoxysStorageError> {
    Ok(BlockNumberView(
        DeoxysBackend::block_number()
            .read()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::BlockNumber))?,
    ))
}

pub fn block_hash_mut<'a>() -> Result<BlockHashViewMut<'a>, DeoxysStorageError> {
    Ok(BlockHashViewMut(
        DeoxysBackend::block_hash()
            .write()
            .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::BlockHash))?,
    ))
}

pub fn block_hash<'a>() -> Result<BlockHashView<'a>, DeoxysStorageError> {
    Ok(BlockHashView(
        DeoxysBackend::block_hash().read().map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::BlockHash))?,
    ))
}

fn conv_contract_identifier(identifier: &ContractAddress) -> &[u8] {
    identifier.0.0.0.as_bytes_ref()
}

fn conv_contract_key(key: &ContractAddress) -> BitVec<u8, Msb0> {
    key.0.0.0.as_bits()[5..].to_owned()
}

fn conv_contract_storage_key(key: &StorageKey) -> BitVec<u8, Msb0> {
    key.0.0.0.as_bits()[5..].to_owned()
}

pub(crate) fn conv_contract_value(value: StarkFelt) -> Felt {
    Felt::from_bytes_be(&value.0)
}

pub(crate) fn conv_class_key(key: &ClassHash) -> BitVec<u8, Msb0> {
    key.0.0.as_bits()[5..].to_owned()
}

fn conv_block_id<DB, H>(
    bonsai: &BonsaiStorage<BasicId, DB, H>,
    block_id: BlockId,
    storage_type: TrieType,
) -> Result<BasicId, DeoxysStorageError>
where
    DB: BonsaiDatabase,
    H: StarkHash + Send + Sync,
{
    match block_id {
        BlockId::Number(block_number) => Ok(BasicId::new(block_number)),
        _ => match bonsai.get_latest_id() {
            Some(id) => Ok(id),
            None => Err(DeoxysStorageError::TrieIdError(storage_type)),
        },
    }
}
