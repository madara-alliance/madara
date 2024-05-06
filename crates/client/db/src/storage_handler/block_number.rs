use mp_felt::Felt252Wrapper;

use super::{DeoxysStorageError, StorageType};
use crate::{Column, DatabaseExt, DeoxysBackend};

pub struct BlockNumberView;

impl BlockNumberView {
    pub fn insert(&mut self, block_hash: &Felt252Wrapper, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockHashToNumber);

        db.put_cf(&column, bincode::serialize(&block_hash)?, bincode::serialize(&block_number)?)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::BlockNumber))
    }
    pub fn get(&self, block_hash: &Felt252Wrapper) -> Result<Option<u64>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockHashToNumber);
        let block_number = db
            .get_cf(&column, bincode::serialize(&block_hash)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?
            .map(|bytes| bincode::deserialize::<u64>(&bytes[..]));

        match block_number {
            Some(Ok(block_number)) => Ok(Some(block_number)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockNumber)),
            None => Ok(None),
        }
    }

    pub fn contains(&self, block_hash: &Felt252Wrapper) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockHashToNumber);

        match db.key_may_exist_cf(&column, bincode::serialize(&block_hash)?) {
            true => Ok(self.get(block_hash)?.is_some()),
            false => Ok(false),
        }
    }
}
