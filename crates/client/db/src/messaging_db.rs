use rocksdb::WriteOptions;
use serde::{Deserialize, Serialize};
use starknet_api::core::Nonce;
use std::collections::BTreeSet;

use crate::error::DbError;
use crate::{Column, DatabaseExt, DeoxysBackend, DeoxysStorageError};

type Result<T, E = DeoxysStorageError> = std::result::Result<T, E>;

pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";
pub const NONCES_PROCESSED: &[u8] = b"NONCES_PROCESSED";

/// Struct to store block number and event_index where L1->L2 Message occured
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LastSyncedEventBlock {
    pub block_number: u64,
    pub event_index: u64,
}

impl LastSyncedEventBlock {
    /// Create a new LastSyncedBlock with block number and event index
    pub fn new(block_number: u64, event_index: u64) -> Self {
        LastSyncedEventBlock { block_number, event_index }
    }
}

/// We add method in DeoxysBackend to be able to handle L1->L2 messaging related data
impl DeoxysBackend {
    /// Retrieves the last stored L1 block data that contains a message from the database.
    ///
    /// This function attempts to fetch the data of the last messaging-related block from the database.
    /// If a block is found, it is retrieved, deserialized, and returned.
    /// Otherwise, a `LastSyncedEventBlock` instance with the `block_number` and `event_index` set to 0 is returned.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(LastSyncedEventBlock))` - If the last synced L1 block with a messaging event is found
    ///   and successfully deserialized.
    /// - `Ok(Some(LastSyncedEventBlock::new(0, 0)))` - If no such block exists in the database.
    /// - `Err(e)` - If there is an error accessing the database or deserializing the block.
    ///
    /// # Errors
    ///
    /// This function returns an error if:
    /// - There is a failure in interacting with the database.
    /// - The block's deserialization fails.
    ///
    /// # Example
    ///
    /// let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
    ///     Ok(Some(blk)) => blk,
    ///     Ok(None) => unreachable!("Should never be None"),
    ///     Err(e) => {
    ///         tracing::error!("⟠ Madara Messaging DB unavailable: {:?}", e);
    ///         return Err(e.into());
    ///     }
    /// };
    ///
    /// # Panics
    ///
    /// This function does not panic.
    pub fn messaging_last_synced_l1_block_with_event(&self) -> Result<Option<LastSyncedEventBlock>> {
        let messaging_column = self.db.get_column(Column::Messaging);
        let Some(res) = self.db.get_cf(&messaging_column, LAST_SYNCED_L1_EVENT_BLOCK)? else {
            return Ok(Some(LastSyncedEventBlock::new(0, 0)));
        };
        let res = bincode::deserialize(&res)?;
        Ok(Some(res))
    }

    /// This function inserts a new block into the messaging column.
    ///
    /// This function retrieves the messaging column and inserts a `LastSyncedEventBlock`
    /// into it.
    ///
    /// # Arguments
    ///
    /// - `last_synced_event_block`: The `LastSyncedEventBlock` instance representing the most recent
    ///   synced L1 block with a messaging event.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the data is correctly inserted into the database.
    /// - `Err(e)` if there is an error accessing the database or serializing the data.
    ///
    /// # Errors
    ///
    /// This function returns an error if:
    /// - There is a failure in interacting with the database.
    /// - The block's serialization fails.
    ///
    /// # Example
    ///
    /// let block_sent = LastSyncedEventBlock::new(l1_block_number.unwrap(), event_index.unwrap());
    /// backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;
    ///
    /// # Panics
    ///
    /// This function does not panic.
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

    /// This function inserts the nonce of the last processed block into the database if it is unique.
    ///
    /// This function takes the nonce of the last processed block and attempts to retrieve the nonce BTreeSet
    /// stored in the `NONCES_PROCESSED` column.
    /// If the column is empty, a new BTreeSet is created, the nonce is inserted into it, and the set is stored in the database.
    /// If there is an existing entry, it is deserialized and checked to see if the nonce is already present.
    /// If the nonce is not in the set, it is inserted into the BTreeSet, which is then serialized and stored in the database,
    /// and the function returns `Ok(true)`.
    /// If the nonce is already present, it means the event has already been processed, so the function returns `Ok(false)`.
    ///
    /// # Arguments
    ///
    /// - `nonce`: The `Nonce` instance representing the nonce of the last messaging event.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the event has not been processed and the nonce has been inserted into the database.
    /// - `Ok(false)` if the event has already been processed.
    /// - `Err(e)` if there is a failure in interacting with the database or in serializing/deserializing the data.
    ///
    /// # Errors
    ///
    /// This function returns an error if:
    /// - There is a failure in interacting with the database.
    /// - There is a failure in serializing or deserializing the data.
    ///
    /// # Example
    ///
    /// match backend.messaging_update_nonces_if_not_used(transaction.nonce) {
    ///     Ok(true) => {}
    ///     Ok(false) => {
    ///         tracing::debug!("⟠ Event already processed: {:?}", transaction);
    ///         return Ok(None);
    ///     }
    ///     Err(e) => {
    ///         tracing::error!("⟠ Unexpected DB error: {:?}", e);
    ///         return Err(e.into());
    ///     }
    /// };
    ///
    /// # Panics
    ///
    /// This function does not panic.
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
