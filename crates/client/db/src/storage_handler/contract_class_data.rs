use crossbeam_skiplist::SkipMap;
use parity_scale_codec::Encode;
use rocksdb::WriteBatchWithTransaction;
use starknet_api::core::ClassHash;

use super::primitives::contract_class::StorageContractClassData;
use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default, Debug)]
pub struct ContractClassDataViewMut(SkipMap<ClassHash, StorageContractClassData>);
pub struct ContractClassDataView;

impl StorageView for ContractClassDataView {
    type KEY = ClassHash;
    type VALUE = StorageContractClassData;

    fn storage_type() -> StorageType {
        StorageType::ContractClassData
    }

    fn storage_column() -> Column {
        Column::ContractClassData
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

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in self.0.into_iter() {
            batch.put_cf(&column, bincode::serialize(&key).unwrap(), value.encode());
        }
        db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassData))
    }
}
