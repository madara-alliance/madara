use std::sync::Arc;

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DB};
use crossbeam_skiplist::SkipMap;
use dp_class::CompiledClass;
use rocksdb::{WriteBatchWithTransaction, WriteOptions};
use starknet_types_core::felt::Felt;

pub struct CompiledContractClassView(Arc<DB>);
#[derive(Debug)]
pub struct CompiledContractClassViewMut(Arc<DB>, SkipMap<Felt, CompiledClass>);

impl CompiledContractClassView {
    pub(crate) fn new(backend: Arc<DB>) -> Self {
        Self(backend)
    }
}
impl CompiledContractClassViewMut {
    pub(crate) fn new(backend: Arc<DB>) -> Self {
        Self(backend, Default::default())
    }
}

impl StorageView for CompiledContractClassView {
    type KEY = Felt;
    type VALUE = CompiledClass;

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = &self.0;
        let column = db.get_column(Column::CompiledContractClass);

        let contract_class_data = db
            .get_cf(&column, bincode::serialize(&class_hash)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::CompiledContractClass))?
            .map(|bytes| bincode::deserialize::<CompiledClass>(&bytes));

        Ok(contract_class_data.transpose()?)
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = &self.0;
        let column = db.get_column(Column::CompiledContractClass);

        match db.key_may_exist_cf(&column, bincode::serialize(&class_hash)?) {
            true => Ok(self.get(class_hash)?.is_some()),
            false => Ok(false),
        }
    }
}

impl StorageViewMut for CompiledContractClassViewMut {
    type KEY = Felt;
    type VALUE = CompiledClass;

    fn insert(&self, class_hash: Self::KEY, contract_class_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.1.insert(class_hash, contract_class_data);
        Ok(())
    }

    fn commit(self, _block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = &self.0;
        let column = db.get_column(Column::CompiledContractClass);
        let _block_number: u32 = _block_number.try_into().map_err(|_| DeoxysStorageError::InvalidBlockNumber)?;

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, value) in self.1.into_iter() {
            batch.put_cf(&column, bincode::serialize(&key)?, bincode::serialize(&value)?);
        }
        let mut write_opt = WriteOptions::default();
        write_opt.disable_wal(true);
        db.write_opt(batch, &write_opt)
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::CompiledContractClass))
    }
}
