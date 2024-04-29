use async_trait::async_trait;
use crossbeam_skiplist::SkipMap;
use rocksdb::WriteBatchWithTransaction;
use starknet_api::core::{ClassHash, CompiledClassHash};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default, Debug)]
pub struct ContractClassHashesViewMut(SkipMap<ClassHash, CompiledClassHash>);
pub struct ContractClassHashesView;

impl StorageView for ContractClassHashesView {
    type KEY = ClassHash;
    type VALUE = CompiledClassHash;

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, super::DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        let compiled_class_hash = db
            .get_cf(&column, bincode::serialize(&class_hash).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClassHashes))?
            .map(|bytes| bincode::deserialize::<CompiledClassHash>(&bytes[..]));

        match compiled_class_hash {
            Some(Ok(compiled_class_hash)) => Ok(Some(compiled_class_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractClassHashes)),
            None => Ok(None),
        }
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, super::DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        match db.key_may_exist_cf(&column, bincode::serialize(&class_hash).unwrap()) {
            true => Ok(self.get(class_hash)?.is_some()),
            false => Ok(false),
        }
    }
}

#[async_trait]
impl StorageViewMut for ContractClassHashesViewMut {
    type KEY = ClassHash;
    type VALUE = CompiledClassHash;

    fn insert(&self, class_hash: Self::KEY, compiled_class_hash: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(class_hash, compiled_class_hash);
        Ok(())
    }

    async fn commit(self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in self.0.into_iter() {
            batch.put_cf(&column, bincode::serialize(&key).unwrap(), bincode::serialize(&value).unwrap());
        }
        db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassHashes))

        // let mut set = JoinSet::new();
        // for (key, value) in self.0.into_iter() {
        //     let db = Arc::clone(db);
        //
        //     set.spawn(async move {
        //         let column = db.get_column(Column::ContractClassHashes);
        //         db.put_cf(&column, key.encode(), value.encode())
        //             .map_err(|_|
        // DeoxysStorageError::StorageCommitError(StorageType::ContractClassHashes))     });
        // }
        //
        // while let Some(res) = set.join_next().await {
        //     res.unwrap()?;
        // }
        //
        // Ok(())
    }
}

impl ContractClassHashesViewMut {
    pub fn commit_sync(self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        for (key, value) in self.0.into_iter() {
            db.put_cf(&column, bincode::serialize(&key).unwrap(), bincode::serialize(&value).unwrap())
                .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassHashes))?
        }

        Ok(())
    }
}
