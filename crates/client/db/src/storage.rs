use std::fmt::Display;
use std::sync::{Arc, RwLock};

use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use lazy_static::lazy_static;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use sp_core::hexdisplay::AsBytesRef;
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon};
use thiserror::Error;

use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

/// Type-safe bonsai storage handler with exclusif acces to the Deoxys backend. Use this to access
/// storage instead of manually querying the bonsai tries.
pub struct StorageHandler;

/// Type safe storage handler for using the backend contract bonsai trie.
///
/// # Usage
///
/// ```
/// StorageHandler::contract().commit(12);
/// ```
///
/// Note that creating a new instance of `ContractTrieHandler` in this way implicitely creates a new
/// `Arc` onto the backend contract storage trie. To store this `Arc` for further use, you can do:
///
/// ```
/// let handler_contract = StorageHandler::contract();
///
/// handler_contract.init();
/// handler_contract.commit(12);
/// ```
pub struct ContractTrieHandler(Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>);

/// Type safe storage handler for using the backend contract storage bonsai trie.
///
/// # Usage
///
/// ```
/// StorageHandler::contract_storage().commit(12);
/// ```
///
/// Note that creating a new instance of `ContractStorageTrieHandler` in this way implicitely
/// creates a new `Arc` onto the backend contract storage trie. To store this `Arc` for further use,
/// you can do:
///
/// ```
/// let handler_contract = StorageHandler::contract_storage();
///
/// handler_contract.init(ContractAddress::from(0u64));
/// handler_contract.commit(12);
/// ```
pub struct ContractStorageTrieHandler(Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>);

/// Type safe storage handler for using the backend class bonsai trie.
///
/// # Usage
///
/// ```
/// StorageHandler::class().commit(12);
/// ```
///
/// Note that creating a new instance of `ClassTrieHandler` in this way implicitely creates a new
/// `Arc` onto the backend contract storage trie. To store this `Arc` for further use, you can do:
///
/// ```
/// let handler_contract = StorageHandler::class();
///
/// handler_contract.init();
/// handler_contract.commit(12);
/// ```
pub struct ClassTrieHandler(Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Poseidon>>>);

#[derive(Debug)]
pub enum StorageType {
    Contract,
    ContractStorage,
    Class,
}

impl Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let storage_type = match self {
            StorageType::Contract => "contract trie",
            StorageType::ContractStorage => "contract storage trie",
            StorageType::Class => "class trie",
        };

        write!(f, "{storage_type}")
    }
}

#[derive(Error, Debug)]
pub enum DeoxysStorageError {
    #[error("failed to insert data into {0}")]
    StorageInsertionError(StorageType),
    #[error("failed to retrive data from {0}")]
    StorageRetrievalError(StorageType),
    #[error("failed to initialize trie for {0}")]
    TrieInitError(StorageType),
    #[error("failed to compute trie root for {0}")]
    TrieRootError(StorageType),
    #[error("failed to commit to {0}")]
    TrieCommitError(StorageType),
}

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = "0xcontract".as_bytes();
    pub const CLASS: &[u8] = "0xclass".as_bytes();
    pub const TRANSACTION: &[u8] = "0xtransaction".as_bytes();
    pub const EVENT: &[u8] = "0xevent".as_bytes();
}

impl StorageHandler {
    pub fn contract() -> ContractTrieHandler {
        ContractTrieHandler(Arc::clone(DeoxysBackend::bonsai_contract()))
    }

    pub fn contract_storage() -> ContractStorageTrieHandler {
        ContractStorageTrieHandler(Arc::clone(DeoxysBackend::bonsai_storage()))
    }

    pub fn class() -> ClassTrieHandler {
        ClassTrieHandler(Arc::clone(DeoxysBackend::bonsai_class()))
    }
}

impl ContractTrieHandler {
    pub fn insert(&mut self, key: &ContractAddress, value: Felt) -> Result<(), DeoxysStorageError> {
        let key = conv_contract_key(key);

        self.0.write().unwrap().insert(bonsai_identifier::CONTRACT, &key, &value).expect("show not fail lol");

        Ok(())
    }

    pub fn get(&self, key: &ContractAddress) -> Result<Option<Felt>, DeoxysStorageError> {
        let key = conv_contract_key(key);

        let value = self
            .0
            .read()
            .unwrap()
            .get(bonsai_identifier::CONTRACT, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))?;

        Ok(value)
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Contract))?;

        Ok(())
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        self.0
            .write()
            .unwrap()
            .init_tree(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Contract))?;

        Ok(())
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        let root_hash = self
            .0
            .read()
            .unwrap()
            .root_hash(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieRootError(StorageType::Contract))?;

        Ok(root_hash)
    }
}

impl ContractStorageTrieHandler {
    pub fn insert(
        &mut self,
        identifier: &ContractAddress,
        key: &StorageKey,
        value: StarkFelt,
    ) -> Result<(), DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);
        let value = conv_contract_value(value);

        self.0.write().unwrap().insert(identifier, &key, &value).unwrap();
        Ok(())
    }

    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        let value = self
            .0
            .read()
            .unwrap()
            .get(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?;

        Ok(value)
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::ContractStorage))?;

        Ok(())
    }

    pub fn init(&mut self, identifier: &ContractAddress) -> Result<(), DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        self.0
            .write()
            .unwrap()
            .init_tree(identifier)
            .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::ContractStorage))?;

        Ok(())
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let root_hash = self
            .0
            .read()
            .unwrap()
            .root_hash(identifier)
            .map_err(|_| DeoxysStorageError::TrieRootError(StorageType::ContractStorage))?;

        Ok(root_hash)
    }
}

lazy_static! {
    static ref CONTRACT_CLASS_HASH_VERSION: FieldElement =
        FieldElement::from_byte_slice_be("CONTRACT_CLASS_LEAF_V0".as_bytes()).unwrap();
}

impl ClassTrieHandler {
    pub fn insert(&mut self, key: &ClassHash, value: &CompiledClassHash) -> Result<(), DeoxysStorageError> {
        let compiled_class_hash = conv_compiled_class_hash(value);
        let hash = PoseidonHasher::hash_elements(*CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);

        let key = conv_class_key(key);
        let value = Felt::from_bytes_be(&hash.to_bytes_be());

        self.0
            .write()
            .unwrap()
            .insert(bonsai_identifier::CLASS, &key, &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?;

        Ok(())
    }

    pub fn get(&self, key: &ClassHash) -> Result<Option<Felt>, DeoxysStorageError> {
        let key = conv_class_key(key);
        let value = self
            .0
            .read()
            .unwrap()
            .get(bonsai_identifier::CLASS, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))?;

        Ok(value)
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Class))?;

        Ok(())
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        self.0
            .write()
            .unwrap()
            .init_tree(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Class))?;

        Ok(())
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        let root_hash = self
            .0
            .read()
            .unwrap()
            .root_hash(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieRootError(StorageType::Class))?;

        Ok(root_hash)
    }
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

fn conv_contract_value(value: StarkFelt) -> Felt {
    Felt::from_bytes_be(&value.0)
}

fn conv_class_key(key: &ClassHash) -> BitVec<u8, Msb0> {
    key.0.0.as_bits()[5..].to_owned()
}

fn conv_compiled_class_hash(compiled_class_hash: &CompiledClassHash) -> FieldElement {
    FieldElement::from_bytes_be(&compiled_class_hash.0.0).unwrap()
}
