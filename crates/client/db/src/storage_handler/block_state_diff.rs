use starknet_core::types::StateDiff;

use super::{DeoxysStorageError, StorageType};
use crate::{Column, DatabaseExt, DeoxysBackend};

pub struct BlockStateDiffView;

impl BlockStateDiffView {
    pub fn insert(&mut self, block_number: u64, state_diff: StateDiff) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockStateDiff);

        db.put_cf(&column, bincode::serialize(&block_number).unwrap(), bincode::serialize(&state_diff).unwrap())
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::BlockStateDiff))
    }

    pub fn get(&self, block_number: u64) -> Result<Option<StateDiff>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockStateDiff);

        let state_diff = db
            .get_cf(&column, bincode::serialize(&block_number).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockStateDiff))?
            .map(|bytes| bincode::deserialize::<StateDiff>(&bytes[..]));

        match state_diff {
            Some(Ok(state_diff)) => Ok(Some(state_diff)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockStateDiff)),
            None => Ok(None),
        }
    }

    pub fn contains(&self, block_number: u64) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockStateDiff);

        match db.key_may_exist_cf(&column, bincode::serialize(&block_number).unwrap()) {
            true => Ok(self.get(block_number)?.is_some()),
            false => Ok(false),
        }
    }
}
