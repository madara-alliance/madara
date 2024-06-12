use crossbeam_skiplist::SkipMap;
use rocksdb::WriteBatchWithTransaction;
use starknet_api::core::ClassHash;
use parity_scale_codec::{Decode, Encode};

use super::primitives::contract_class::StorageContractClassData;
use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default, Debug)]
pub struct ContractClassDataViewMut(SkipMap<ClassHash, StorageContractClassData>);
pub struct ContractClassDataView;

impl StorageView for ContractClassDataView {
    type KEY = ClassHash;
    type VALUE = StorageContractClassData;

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        let contract_class_data = db
            .get_cf(&column, bincode::serialize(&class_hash)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClassData))?
            .map(|bytes| StorageContractClassData::decode(&mut &bytes[..]));

        match contract_class_data {
            Some(Ok(contract_class_data)) => Ok(Some(contract_class_data)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageSerdeError),
            None => Ok(None),
        }
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        match db.key_may_exist_cf(&column, bincode::serialize(&class_hash)?) {
            true => Ok(self.get(class_hash)?.is_some()),
            false => Ok(false),
        }
    }
}

impl StorageViewMut for ContractClassDataViewMut {
    type KEY = ClassHash;
    type VALUE = StorageContractClassData;

    fn insert(&self, class_hash: Self::KEY, contract_class_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(class_hash, contract_class_data);
        Ok(())
    }

    fn commit(self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);
        let _block_number: u32 = _block_number.try_into().map_err(|_| DeoxysStorageError::InvalidBlockNumber)?;

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in self.0.into_iter() {
            batch.put_cf(&column, bincode::serialize(&key)?, value.encode());
        }
        db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassData))
    }
}
