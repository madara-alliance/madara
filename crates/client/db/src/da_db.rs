use std::sync::Arc;

// Substrate
use parity_scale_codec::{Decode, Encode};
// Starknet
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkFelt;
use starknet_api::state::ThinStateDiff;
use uuid::Uuid;

use crate::{Column, DatabaseExt, DbError, DB};

// The fact db stores DA facts that need to be written to L1
pub struct DaDb {
    pub(crate) db: Arc<DB>,
}

// TODO: purge old cairo job keys
impl DaDb {
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn state_diff(&self, block_hash: &BlockHash) -> Result<ThinStateDiff, DbError> {
        let column = self.db.get_column(Column::Da);

        match self.db.get_cf(&column, block_hash.0.bytes())? {
            Some(raw) => Ok(ThinStateDiff::decode(&mut &raw[..])?),
            None => Err(DbError::ValueNotInitialized(Column::Da, block_hash.to_string())),
        }
    }

    pub fn store_state_diff(&self, block_hash: &BlockHash, diff: &ThinStateDiff) -> Result<(), DbError> {
        let column = self.db.get_column(Column::Da);

        self.db.put_cf(&column, block_hash.0.bytes(), &diff.encode())?;
        Ok(())
    }

    pub fn cairo_job(&self, block_hash: &BlockHash) -> Result<Option<Uuid>, DbError> {
        let column = self.db.get_column(Column::Da);

        match self.db.get_cf(&column, block_hash.0.bytes())? {
            Some(raw) => Ok(Some(Uuid::from_slice(&raw[..])?)),
            None => Ok(None),
        }
    }

    pub fn update_cairo_job(&self, block_hash: &BlockHash, job_id: Uuid) -> Result<(), DbError> {
        let column = self.db.get_column(Column::Da);

        self.db.put_cf(&column, block_hash.0.bytes(), &job_id.into_bytes())?;
        Ok(())
    }

    pub fn last_proved_block(&self) -> Result<BlockHash, DbError> {
        let column = self.db.get_column(Column::Da);

        match self.db.get_cf(&column, crate::static_keys::LAST_PROVED_BLOCK)? {
            Some(raw) => {
                let felt = StarkFelt::decode(&mut &raw[..])?;
                Ok(BlockHash(felt))
            }
            None => Err(DbError::ValueNotInitialized(
                Column::Da,
                String::from_utf8(crate::static_keys::LAST_PROVED_BLOCK.to_vec()).expect("unreachable"),
            )),
        }
    }

    pub fn update_last_proved_block(&self, block_hash: &BlockHash) -> Result<(), DbError> {
        let column = self.db.get_column(Column::Da);

        self.db.put_cf(&column, crate::static_keys::LAST_PROVED_BLOCK, &block_hash.0.encode())?;
        Ok(())
    }
}
