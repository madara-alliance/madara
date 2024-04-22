use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::{ContractAddress, Nonce};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::bonsai_db::BonsaiDb;

pub struct NonceView<'a>(pub(crate) RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);
pub struct NonceViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for NonceView<'_> {
    type KEY = ContractAddress;

    type VALUE = Nonce;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, super::DeoxysStorageError> {
        let nonce = self
            .0
            .get(&key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Nonce))?
            .map(|bytes| Nonce::decode(&mut &bytes[..]));

        match nonce {
            Some(Ok(nonce)) => Ok(Some(nonce)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::Nonce)),
            None => Ok(None),
        }
    }

    fn get_at(
        self,
        contract_address: &Self::KEY,
        block_number: u64,
    ) -> Result<Option<Self::VALUE>, super::DeoxysStorageError> {
        let nonce = self
            .0
            .get_at(&key(contract_address), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Nonce))?
            .map(|bytes| Nonce::decode(&mut &bytes[..]));

        match nonce {
            Some(Ok(nonce)) => Ok(Some(nonce)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::Nonce)),
            None => Ok(None),
        }
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, super::DeoxysStorageError> {
        self.0
            .contains(&key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Nonce))
    }
}

impl StorageViewMut for NonceViewMut<'_> {
    type KEY = ContractAddress;

    type VALUE = Nonce;

    fn insert(&mut self, contract_address: &Self::KEY, nonce: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(&key(contract_address), &nonce.encode());
        Ok(())
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Nonce))
    }
}

impl StorageViewRevetible for NonceViewMut<'_> {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::Nonce, block_number))
    }
}

fn key(contract_address: &ContractAddress) -> BitVec<u8, Msb0> {
    contract_address.bytes().view_bits()[5..].to_owned()
}
