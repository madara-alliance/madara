use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Decode, Encode};

use super::{DeoxysStorageError, StorageType};
use crate::{Column, DatabaseExt, DeoxysBackend};

pub struct BlockNumberView;

impl BlockNumberView {
    pub fn get(self, block_hash: &Felt252Wrapper) -> Result<Option<u64>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockHashToNumber);
        let block_number = db
            .get_cf(&column, block_hash.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?
            .map(|bytes| u64::decode(&mut &bytes[..]));

        match block_number {
            Some(Ok(block_number)) => Ok(Some(block_number)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockNumber)),
            None => Ok(None),
        }
    }

    pub fn contains(self, block_hash: &Felt252Wrapper) -> Result<bool, DeoxysStorageError> {
        Ok(matches!(self.get(block_hash)?, Some(_)))
    }

    pub fn insert(&mut self, block_hash: &Felt252Wrapper, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::BlockHashToNumber);

        db.put_cf(&column, block_hash.encode(), block_number.encode())
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::BlockNumber))
    }
}
