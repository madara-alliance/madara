use mp_felt::Felt252Wrapper;

use super::{DeoxysStorageError, StorageType};
use crate::{Column, DatabaseExt, DeoxysBackend};

pub struct BlockHashView;

impl BlockHashView {
    pub fn insert(&mut self, block_number: u64, block_hash: &Felt252Wrapper) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockNumberToHash);
        db.put_cf(&column, bincode::serialize(&block_number).unwrap(), bincode::serialize(&block_hash).unwrap())
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::BlockHash))
    }

    pub fn get(&self, block_number: u64) -> Result<Option<Felt252Wrapper>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockNumberToHash);
        let block_hash = db
            .get_cf(&column, bincode::serialize(&block_number).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?
            .map(|bytes| bincode::deserialize::<Felt252Wrapper>(&bytes[..]));

        match block_hash {
            Some(Ok(block_hash)) => Ok(Some(block_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockHash)),
            None => Ok(None),
        }
    }

    pub fn contains(&self, block_number: u64) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockNumberToHash);

        match db.key_may_exist_cf(&column, bincode::serialize(&block_number).unwrap()) {
            true => Ok(self.get(block_number)?.is_some()),
            false => Ok(false),
        }
    }
}
