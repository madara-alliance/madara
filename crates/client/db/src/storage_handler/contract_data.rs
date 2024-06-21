use std::ops::Deref;

use starknet_api::core::{ClassHash, ContractAddress, Nonce};

use super::history::{AsHistoryView, HistoryView, HistoryViewMut};
use super::DeoxysStorageError;
use crate::Column;

// NB: Column cfs needs prefix extractor of this length during creation
pub(crate) const CONTRACT_CLASS_HASH_PREFIX_EXTRACTOR: usize = 32;
pub(crate) const CONTRACT_NONCES_PREFIX_EXTRACTOR: usize = 32;

#[derive(Debug)]
pub struct ContractAddressK([u8; 32]);
impl From<ContractAddress> for ContractAddressK {
    fn from(value: ContractAddress) -> Self {
        Self(value.0.key().bytes().try_into().unwrap())
    }
}
impl Deref for ContractAddressK {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Class hash storage

pub struct ContractClassAsHistory;
impl AsHistoryView for ContractClassAsHistory {
    type Key = ContractAddress;
    type KeyBin = ContractAddressK;
    type T = ClassHash;
    fn column() -> Column {
        Column::ContractToClassHashes
    }
}

pub type ContractClassView = HistoryView<ContractClassAsHistory>;
pub type ContractClassViewMut = HistoryViewMut<ContractClassAsHistory>;

// Nonce storage

pub struct ContractNoncesAsHistory;
impl AsHistoryView for ContractNoncesAsHistory {
    type Key = ContractAddress;
    type KeyBin = ContractAddressK;
    type T = Nonce;
    fn column() -> Column {
        Column::ContractToNonces
    }
}

pub type ContractNoncesView = HistoryView<ContractNoncesAsHistory>;
pub type ContractNoncesViewMut = HistoryViewMut<ContractNoncesAsHistory>;

impl ContractClassView {
    pub fn is_contract_deployed_at(
        &self,
        contract_address: &ContractAddress,
        block_number: u64,
    ) -> Result<bool, DeoxysStorageError> {
        Ok(self.get_at(contract_address, block_number)?.is_some())
    }
}
