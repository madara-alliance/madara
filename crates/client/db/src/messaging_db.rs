use rocksdb::WriteOptions;
use serde::{Deserialize, Serialize};
use starknet_api::core::Nonce;
use std::collections::BTreeSet;

use crate::error::DbError;
use crate::{Column, DatabaseExt, DeoxysBackend, DeoxysStorageError};

type Result<T, E = DeoxysStorageError> = std::result::Result<T, E>;

pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";
pub const NONCES_PROCESSED: &[u8] = b"NONCES_PROCESSED";

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
        last_synced_event_block: LastSyncedEventBlock,
    ) -> Result<(), DbError> {
        let messaging_column = self.db.get_column(Column::Messaging);
        let mut writeopts = WriteOptions::default(); // todo move that in db
        writeopts.disable_wal(true);
        self.db.put_cf_opt(
            &messaging_column,
            LAST_SYNCED_L1_EVENT_BLOCK,
            bincode::serialize(&last_synced_event_block)?,
            &writeopts,
        )?;
        Ok(())
    }

    pub fn messaging_update_nonces_if_not_used(&self, nonce: Nonce) -> Result<bool, DbError> {
        let messaging_column = self.db.get_column(Column::Messaging);
        let Some(res) = self.db.get_cf(&messaging_column, NONCES_PROCESSED)? else {
            // No nonces have been processed yet, we will just create the BTreeSet
            let mut writeopts = WriteOptions::default(); // todo move that in db
            writeopts.disable_wal(true);

            let mut btree_set = BTreeSet::new();
            btree_set.insert(nonce);

            self.db.put_cf_opt(&messaging_column, NONCES_PROCESSED, bincode::serialize(&btree_set)?, &writeopts)?;
            return Ok(true);
        };

        let mut nonce_tree: BTreeSet<Nonce> = bincode::deserialize(&res)?;
        if nonce_tree.contains(&nonce) {
            // Nonce is already used, return false
            Ok(false)
        } else {
            // Nonce isn't being used yet, add it to our db and return true
            nonce_tree.insert(nonce);

            let mut writeopts = WriteOptions::default(); // todo move that in db
            writeopts.disable_wal(true);

            self.db.put_cf_opt(&messaging_column, NONCES_PROCESSED, bincode::serialize(&nonce_tree)?, &writeopts)?;
            Ok(true)
        }
    }
}
