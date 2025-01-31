use crate::error::DbError;
use crate::{Column, DatabaseExt, MadaraBackend, MadaraStorageError};
use rocksdb::IteratorMode;
use serde::{Deserialize, Serialize};
use starknet_api::core::Nonce;

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";

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

/// We add method in MadaraBackend to be able to handle L1->L2 messaging related data
impl MadaraBackend {
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
    #[tracing::instrument(skip(self), fields(module = "L1DB"))]
    pub fn messaging_last_synced_l1_block_with_event(&self) -> Result<Option<LastSyncedEventBlock>> {
        let messaging_column = self.db.get_column(Column::L1Messaging);
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
    #[tracing::instrument(skip(self), fields(module = "L1DB"))]
    pub fn messaging_update_last_synced_l1_block_with_event(
        &self,
        last_synced_event_block: LastSyncedEventBlock,
    ) -> Result<(), DbError> {
        let messaging_column = self.db.get_column(Column::L1Messaging);
        self.db.put_cf_opt(
            &messaging_column,
            LAST_SYNCED_L1_EVENT_BLOCK,
            bincode::serialize(&last_synced_event_block)?,
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    #[tracing::instrument(skip(self, nonce), fields(module = "L1DB"))]
    pub fn has_l1_messaging_nonce(&self, nonce: Nonce) -> Result<bool> {
        let nonce_column = self.db.get_column(Column::L1MessagingNonce);
        Ok(self.db.get_pinned_cf(&nonce_column, bincode::serialize(&nonce)?)?.is_some())
    }

    #[tracing::instrument(skip(self, nonce), fields(module = "L1DB"))]
    pub fn set_l1_messaging_nonce(&self, nonce: Nonce) -> Result<(), DbError> {
        let nonce_column = self.db.get_column(Column::L1MessagingNonce);
        self.db.put_cf_opt(
            &nonce_column,
            bincode::serialize(&nonce)?,
            /* empty value */ [],
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    /// Retrieve the latest L1 messaging [Nonce] if one is available, otherwise
    /// returns [None].
    pub fn get_l1_messaging_nonce_latest(&self) -> Result<Option<Nonce>, MadaraStorageError> {
        let nonce_column = self.db.get_column(Column::L1MessagingNonce);
        let mut iter = self.db.iterator_cf(&nonce_column, IteratorMode::End);
        let nonce = iter.next().transpose()?.map(|(bytes, _)| bincode::deserialize(&bytes)).transpose()?;
        Ok(nonce)
    }
}
