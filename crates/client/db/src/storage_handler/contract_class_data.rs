use async_trait::async_trait;
use crossbeam_skiplist::SkipMap;
use parity_scale_codec::{Decode, Encode};
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

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        let contract_class_data = db
            .get_cf(&column, bincode::serialize(&class_hash).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClassData))?
            .map(|bytes| StorageContractClassData::decode(&mut &bytes[..]));

        match contract_class_data {
            Some(Ok(contract_class_data)) => Ok(Some(contract_class_data)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::Class)),
            None => Ok(None),
        }
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        match db.key_may_exist_cf(&column, bincode::serialize(&class_hash).unwrap()) {
            true => Ok(self.get(class_hash)?.is_some()),
            false => Ok(false),
        }
    }
}

#[async_trait]
impl StorageViewMut for ContractClassDataViewMut {
    type KEY = ClassHash;
    type VALUE = StorageContractClassData;

    fn insert(&self, class_hash: Self::KEY, contract_class_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(class_hash, contract_class_data);
        Ok(())
    }

    async fn commit(self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassData);

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in self.0.into_iter() {
            batch.put_cf(&column, bincode::serialize(&key).unwrap(), value.encode());
        }
        db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassData))

        // let mut set = JoinSet::new();
        // for (key, value) in self.0.into_iter() {
        //     let db = Arc::clone(db);
        //
        //     set.spawn(async move {
        //         let colum = db.get_column(Column::ContractClassData);
        //         db.put_cf(&colum, key.encode(), value.encode())
        //             .map_err(|_|
        // DeoxysStorageError::StorageInsertionError(StorageType::ContractClassData))     });
        // }
        //
        // while let Some(res) = set.join_next().await {
        //     res.unwrap()?;
        // }
    }
}
