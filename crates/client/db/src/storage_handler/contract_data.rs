use std::collections::BTreeMap;

use crossbeam_skiplist::SkipMap;
use mp_contract::class::StorageContractData;
use parity_scale_codec::{Decode, Encode};
use rocksdb::IteratorMode;
use starknet_api::core::{ClassHash, ContractAddress};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default)]
pub struct ContractDataViewMut(SkipMap<ContractAddress, StorageContractData>);
pub struct ContractDataView;

impl StorageView for ContractDataView {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let tree: BTreeMap<u64, StorageContractData> = match db
            .get_cf(&column, contract_address.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => BTreeMap::new(),
        };

        Ok(tree.last_key_value().map(|(_k, v)| v).cloned())
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self.get(contract_address)?.is_some())
    }
}

impl ContractDataView {
    // FIXME: retrieve value if it does not exist at the given block number
    pub fn get_at(
        self,
        contract_address: &ContractAddress,
        block_number: u64,
    ) -> Result<Option<StorageContractData>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let tree: BTreeMap<u64, StorageContractData> = match db
            .get_cf(&column, contract_address.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => BTreeMap::new(),
        };

        Ok(tree.get(&block_number).cloned())
    }
}

impl StorageView for ContractDataViewMut {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        if let Some(entry) = self.0.get(contract_address) {
            return Ok(Some(entry.value().clone()));
        }

        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let tree: BTreeMap<u64, StorageContractData> = match db
            .get_cf(&column, contract_address.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => BTreeMap::new(),
        };

        Ok(tree.last_key_value().map(|(_k, v)| v).cloned())
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self.get(contract_address)?.is_some())
    }
}

impl StorageViewMut for ContractDataViewMut {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn insert(&self, contract_address: Self::KEY, contract_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(contract_address, contract_data);
        Ok(())
    }

    fn commit(&self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        for entry in self.0.iter() {
            let mut tree: BTreeMap<u64, StorageContractData> = match db
                .get_cf(&column, entry.key().encode())
                .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
            {
                Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                    .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
                None => BTreeMap::new(),
            };

            tree.insert(block_number, entry.value().clone());
            db.put_cf(&column, entry.key().encode(), tree.encode())
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::ContractData))?;
        }

        Ok(())
    }
}

impl StorageViewRevetible for ContractDataViewMut {
    fn revert_to(&self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let iterator = db.iterator_cf(&column, IteratorMode::Start);

        for data in iterator {
            let (key, value) =
                data.map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?;
            let mut tree: BTreeMap<u64, ClassHash> = BTreeMap::decode(&mut &value[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?;

            tree.retain(|&k, _| k <= block_number);

            db.put_cf(&column, key, tree.encode())
                .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractData, block_number))?;
        }

        Ok(())
    }
}
