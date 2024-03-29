use std::sync::Arc;

// Substrate
use parity_scale_codec::{Decode, Encode};

use crate::error::DbError;
use crate::{Column, DB, DatabaseExt};

pub struct MessagingDb {
    pub(crate) db: Arc<DB>,
}

#[derive(Encode, Decode)]
pub struct LastSyncedEventBlock {
    pub block_number: u64,
    pub event_index: u64,
}

impl LastSyncedEventBlock {
    pub fn new(block_number: u64, event_index: u64) -> Self {
        LastSyncedEventBlock { block_number, event_index }
    }
}

impl MessagingDb {
    pub(crate) fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn last_synced_l1_block_with_event(&self) -> Result<LastSyncedEventBlock, DbError> {
        let column = self.db.get_column(Column::Messaging);

        match self.db.get_cf(&column, crate::static_keys::LAST_SYNCED_L1_EVENT_BLOCK)? {
            Some(raw) => Ok(LastSyncedEventBlock::decode(&mut &raw[..])?),
            None => Ok(LastSyncedEventBlock::new(0, 0)),
        }
    }

    pub fn update_last_synced_l1_block_with_event(
        &self,
        last_synced_event_block: &LastSyncedEventBlock,
    ) -> Result<(), DbError> {
        let column = self.db.get_column(Column::Messaging);

        self.db.put_cf(
            &column,
            crate::static_keys::LAST_SYNCED_L1_EVENT_BLOCK,
            &last_synced_event_block.encode(),
        )?;
        Ok(())
    }
}
