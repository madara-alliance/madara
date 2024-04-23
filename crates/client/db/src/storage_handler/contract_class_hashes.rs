use crossbeam_skiplist::SkipMap;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::{ClassHash, CompiledClassHash};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default)]
pub struct ContractClassHashesViewMut(SkipMap<ClassHash, CompiledClassHash>);
pub struct ContractClassHashesView;

impl StorageView for ContractClassHashesView {
    type KEY = ClassHash;
    type VALUE = CompiledClassHash;

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, super::DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        let compiled_class_hash = db
            .get_cf(&column, class_hash.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClassHashes))?
            .map(|bytes| CompiledClassHash::decode(&mut &bytes[..]));

        match compiled_class_hash {
            Some(Ok(compiled_class_hash)) => Ok(Some(compiled_class_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractClassHashes)),
            None => Ok(None),
        }
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, super::DeoxysStorageError> {
        Ok(self.get(class_hash)?.is_some())
    }
}

impl StorageViewMut for ContractClassHashesViewMut {
    type KEY = ClassHash;
    type VALUE = CompiledClassHash;

    fn insert(&self, class_hash: Self::KEY, compiled_class_hash: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(class_hash, compiled_class_hash);
        Ok(())
    }

    fn commit(&self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractClassHashes);

        for entry in self.0.iter() {
            db.put_cf(&column, entry.key().encode(), entry.value().encode())
                .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClassHashes))?;
        }

        Ok(())
    }
}
