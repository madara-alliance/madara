use std::fmt::Display;
use std::sync::RwLockReadGuard;

use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use sp_core::hexdisplay::AsBytesRef;
use starknet_api::api_core::{ClassHash, ContractAddress};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon};
use thiserror::Error;

use crate::bonsai_db::{BonsaiDb, BonsaiTransaction};
use crate::DeoxysBackend;

/// Type-safe bonsai storage handler with exclusif acces to the Deoxys backend. Use this to access
/// storage instead of manually querying the bonsai tries.
pub struct StorageHandler;

pub struct ContractTrieMut(BonsaiStorage<BasicId, BonsaiTransaction<'static>, Pedersen>);

pub struct ContractTrieView<'a>(RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);

pub struct ContractStorageTrieMut(BonsaiStorage<BasicId, BonsaiTransaction<'static>, Pedersen>);

pub struct ContractStorageTrieView<'a>(RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);

pub struct ClassTrieMut(BonsaiStorage<BasicId, BonsaiTransaction<'static>, Poseidon>);

pub struct ClassTrieView<'a>(RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>>);

#[derive(Debug)]
pub enum StorageType {
    Contract,
    ContractStorage,
    Class,
}

impl Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
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
    #[error("failed to retrieve storage view for {0}")]
    StoraveViewError(StorageType),
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
    #[error("failed to merge transactional state back into {0}")]
    TrieMergeError(StorageType),
}

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = "0xcontract".as_bytes();
    pub const CLASS: &[u8] = "0xclass".as_bytes();
    pub const TRANSACTION: &[u8] = "0xtransaction".as_bytes();
    pub const EVENT: &[u8] = "0xevent".as_bytes();
}

impl StorageHandler {
    #[rustfmt::skip]
    pub fn contract_mut(block_number: u64) -> Result<ContractTrieMut, DeoxysStorageError> {
		let bonsai_contract = DeoxysBackend::bonsai_contract().read().unwrap();
		let config = bonsai_contract.get_config();
        let transactional_storage =
			bonsai_contract.get_transactional_state(BasicId::new(block_number), config);

        match transactional_storage {
            Ok(Some(transactional_storage)) => {
                Ok(ContractTrieMut(transactional_storage))
            }
            _ => {
				Err(DeoxysStorageError::StoraveViewError(StorageType::Contract))
			}
        }
    }

    pub fn contract<'a>() -> Result<ContractTrieView<'a>, DeoxysStorageError> {
        Ok(ContractTrieView(
            DeoxysBackend::bonsai_contract()
                .read()
                .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::Contract))?,
        ))
    }

    #[rustfmt::skip]
    pub fn contract_storage_mut(block_number: u64) -> Result<ContractStorageTrieMut, DeoxysStorageError> {
		let bonsai_storage = DeoxysBackend::bonsai_storage().read().unwrap();
		let config = bonsai_storage.get_config();
        let transactional_storage =
			bonsai_storage.get_transactional_state(BasicId::new(block_number), config);

        match transactional_storage {
            Ok(Some(transactional_storage)) => {
				Ok(ContractStorageTrieMut(transactional_storage))
			}
            _ => {
				Err(DeoxysStorageError::StoraveViewError(StorageType::ContractStorage))
			}
        }
    }

    pub fn contract_storage<'a>() -> Result<ContractStorageTrieView<'a>, DeoxysStorageError> {
        Ok(ContractStorageTrieView(
            DeoxysBackend::bonsai_storage()
                .read()
                .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::ContractStorage))?,
        ))
    }

    #[rustfmt::skip]
    pub fn class_mut(block_number: u64) -> Result<ClassTrieMut, DeoxysStorageError> {
        let bonsai_classes = DeoxysBackend::bonsai_class().read().unwrap();
        let config = bonsai_classes.get_config();
        let transactional_storage =
			bonsai_classes.get_transactional_state(BasicId::new(block_number), config);

		match transactional_storage {
			Ok(Some(transactional_storage)) => {
				Ok(ClassTrieMut(transactional_storage))
			},
			_ => {
				Err(DeoxysStorageError::StoraveViewError(StorageType::Class))
			}
		}
    }

    pub fn class<'a>() -> Result<ClassTrieView<'a>, DeoxysStorageError> {
        Ok(ClassTrieView(
            DeoxysBackend::bonsai_class()
                .read()
                .map_err(|_| DeoxysStorageError::StoraveViewError(StorageType::Class))?,
        ))
    }
}

impl ContractTrieMut {
    pub fn update(&mut self, updates: Vec<(&ContractAddress, Felt)>) -> Result<(), DeoxysStorageError> {
        // for (key, value) in updates {
        //     let key = conv_contract_key(key);
        //     self.0
        //         .insert(bonsai_identifier::CONTRACT, &key, &value)
        //         .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        // }

        let mut lock = DeoxysBackend::bonsai_contract().write().unwrap();
        for (key, value) in updates {
            lock.insert(bonsai_identifier::CONTRACT, &conv_contract_key(key), &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        }

        Ok(())
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .transactional_commit(BasicId::new(block_number))
        //     .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Contract))?;

        DeoxysBackend::bonsai_contract()
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Contract))
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(bonsai_identifier::CONTRACT)
        //     .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Class))?;

        DeoxysBackend::bonsai_contract()
            .write()
            .unwrap()
            .init_tree(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Contract))
    }

    #[deprecated = "Waiting on new transactional state implementations in Bonsai Db"]
    pub fn apply_changes(self) -> Result<(), DeoxysStorageError> {
        // Ok(DeoxysBackend::bonsai_contract()
        //     .write()
        //     .unwrap()
        //     .merge(self.0)
        //     .map_err(|_| DeoxysStorageError::TrieMergeError(StorageType::Contract))?)
        Ok(())
    }

    pub fn get(&self, key: &ContractAddress) -> Result<Option<Felt>, DeoxysStorageError> {
        // let key = conv_contract_key(key);
        // Ok(self
        //     .0
        //     .get(bonsai_identifier::CONTRACT, &key)
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))?)

        DeoxysBackend::bonsai_contract()
            .read()
            .unwrap()
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl ContractTrieView<'_> {
    pub fn get(&self, key: &ContractAddress) -> Result<Option<Felt>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0
            .root_hash(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieRootError(StorageType::Contract))
    }
}

impl ContractStorageTrieMut {
    pub fn insert(
        &mut self,
        identifier: &ContractAddress,
        key: &StorageKey,
        value: StarkFelt,
    ) -> Result<(), DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);
        let value = conv_contract_value(value);

        // self.0.insert(identifier, &key, &value).unwrap();
        // Ok(())

        DeoxysBackend::bonsai_storage()
            .write()
            .unwrap()
            .insert(identifier, &key, &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::ContractStorage))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .transactional_commit(BasicId::new(block_number))
        //     .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::ContractStorage))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_storage()
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::ContractStorage))
    }

    pub fn init(&mut self, identifier: &ContractAddress) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(conv_contract_identifier(identifier))
        //     .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::ContractStorage))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_storage()
            .write()
            .unwrap()
            .init_tree(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::ContractStorage))
    }

    #[deprecated = "Waiting on new transactional state implementations in Bonsai Db"]
    pub fn apply_changes(self) -> Result<(), DeoxysStorageError> {
        // Ok(DeoxysBackend::bonsai_storage()
        //     .write()
        //     .unwrap()
        //     .merge(self.0)
        //     .map_err(|_| DeoxysStorageError::TrieMergeError(StorageType::ContractStorage))?)
        Ok(())
    }

    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        // let identifier = conv_contract_identifier(identifier);
        // let key = conv_contract_storage_key(key);
        //
        // Ok(self
        //     .0
        //     .get(identifier, &key)
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?)

        DeoxysBackend::bonsai_storage()
            .read()
            .unwrap()
            .get(conv_contract_identifier(identifier), &conv_contract_storage_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(&self, identifier: &ContractAddress) -> Result<Vec<(Felt, Felt)>, DeoxysStorageError> {
        Ok(self
            .0
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (Felt::from_bytes_be_slice(&k), Felt::from_bytes_be_slice(&v)))
            .collect())
    }
}

impl ContractStorageTrieView<'_> {
    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        self.0
            .get(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        self.0
            .root_hash(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieRootError(StorageType::ContractStorage))
    }
}

impl ClassTrieMut {
    pub fn update(&mut self, updates: Vec<(&ClassHash, FieldElement)>) -> Result<(), DeoxysStorageError> {
        // for (key, value) in updates {
        //     let key = conv_class_key(key);
        //     let value = Felt::from_bytes_be(&value.to_bytes_be());
        //     self.0
        //         .insert(bonsai_identifier::CLASS, &key, &value)
        //         .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;
        // }

        let mut lock = DeoxysBackend::bonsai_class().write().unwrap();
        for (key, value) in updates {
            let key = conv_class_key(key);
            let value = Felt::from_bytes_be(&value.to_bytes_be());
            lock.insert(bonsai_identifier::CLASS, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;
        }

        Ok(())
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .transactional_commit(BasicId::new(block_number))
        //     .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Class))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_class()
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Class))
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(bonsai_identifier::CLASS)
        //     .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Class))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_class()
            .write()
            .unwrap()
            .init_tree(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Class))
    }

    #[deprecated = "Waiting on new transactional state implementations in Bonsai Db"]
    pub fn apply_changes(self) -> Result<(), DeoxysStorageError> {
        // Ok(DeoxysBackend::bonsai_class()
        //     .write()
        //     .unwrap()
        //     .merge(self.0)
        //     .map_err(|_| DeoxysStorageError::TrieMergeError(StorageType::Class))?)
        Ok(())
    }

    pub fn get(&self, key: &ClassHash) -> Result<Option<Felt>, DeoxysStorageError> {
        // let key = conv_class_key(key);
        //
        // Ok(self
        //     .0
        //     .get(bonsai_identifier::CLASS, &key)
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))?)
        DeoxysBackend::bonsai_class()
            .read()
            .unwrap()
            .get(bonsai_identifier::CLASS, &conv_class_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }
}

impl ClassTrieView<'_> {
    pub fn get(&self, key: &ClassHash) -> Result<Option<Felt>, DeoxysStorageError> {
        let key = conv_class_key(key);

        self.0
            .get(bonsai_identifier::CLASS, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CLASS).map_err(|_| DeoxysStorageError::TrieRootError(StorageType::Class))
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
