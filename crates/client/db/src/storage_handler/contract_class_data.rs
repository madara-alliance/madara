use crossbeam_skiplist::SkipMap;
use mp_contract::class::StorageContractClassData;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::ClassHash;

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default)]
pub struct ContractClassDataViewMut(SkipMap<ClassHash, StorageContractClassData>);
pub struct ContractClassDataView;

impl StorageView for ContractClassDataView {
    type KEY = ClassHash;
    type VALUE = StorageContractClassData;

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        let contract_class_data = db
            .get_cf(&column, class_hash.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClassData))?
            .map(|bytes| StorageContractClassData::decode(&mut &bytes[..]));

        match contract_class_data {
            Some(Ok(contract_class_data)) => Ok(Some(contract_class_data)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::Class)),
            None => Ok(None),
        }
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self.get(class_hash)?.is_some())
    }
}

impl StorageViewMut for ContractClassDataViewMut {
    type KEY = ClassHash;
    type VALUE = StorageContractClassData;

    fn insert(&self, class_hash: Self::KEY, contract_class_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(class_hash, contract_class_data);
        Ok(())
    }

    fn commit(&self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        for entry in self.0.iter() {
            db.put_cf(&column, entry.key().encode(), entry.value().encode())
                .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClassData))?;
        }

        Ok(())
    }
}
