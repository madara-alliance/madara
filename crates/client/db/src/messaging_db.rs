use rocksdb::WriteOptions;
use serde::{Deserialize, Serialize};

use crate::error::DbError;
use crate::{Column, DatabaseExt, DeoxysBackend, DeoxysStorageError};

type Result<T, E = DeoxysStorageError> = std::result::Result<T, E>;

pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LastSyncedEventBlock {
    pub block_number: u64,
    pub event_index: u64,
}

impl LastSyncedEventBlock {
    pub fn new(block_number: u64, event_index: u64) -> Self {
        LastSyncedEventBlock { block_number, event_index }
    }
}

impl DeoxysBackend {
    pub fn messaging_last_synced_l1_block_with_event(&self) -> Result<Option<LastSyncedEventBlock>> {
        let messaging_column = self.db.get_column(Column::Messaging);
        let Some(res) = self.db.get_cf(&messaging_column, LAST_SYNCED_L1_EVENT_BLOCK)? else {
            return Ok(Some(LastSyncedEventBlock::new(0, 0)));
        };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    pub fn messaging_update_last_synced_l1_block_with_event(
        &self,
        last_synced_event_block: &LastSyncedEventBlock,
    ) -> Result<(), DbError> {
        let messaging_column = self.db.get_column(Column::Messaging);
        let mut writeopts = WriteOptions::default(); // todo move that in db
        writeopts.disable_wal(true);
        self.db.put_cf_opt(
            &messaging_column,
            LAST_SYNCED_L1_EVENT_BLOCK,
            bincode::serialize(last_synced_event_block)?,
            &writeopts,
        )?;
        Ok(())
    }
}
