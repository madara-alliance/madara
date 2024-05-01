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

    fn storage_type() -> StorageType {
        StorageType::ContractClassHashes
    }

    fn storage_column() -> Column {
        Column::ContractClassHashes
    }
}

impl StorageViewMut for ContractClassHashesViewMut {
    type KEY = ClassHash;
    type VALUE = CompiledClassHash;

    fn insert(&self, class_hash: Self::KEY, compiled_class_hash: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(class_hash, compiled_class_hash);
        Ok(())
    }

    fn commit(self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in self.0.into_iter() {
            batch.put_cf(&column, bincode::serialize(&key).unwrap(), bincode::serialize(&value).unwrap());
        }
        db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassHashes))
    }
}
