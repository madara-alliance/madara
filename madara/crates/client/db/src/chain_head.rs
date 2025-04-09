use crate::{Column, MadaraBackend, MadaraStorageError};
use crate::{DatabaseExt, DB};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
#[serde(transparent)]
pub struct BlockNStatus(AtomicU64);

impl BlockNStatus {
    pub fn current(&self) -> Option<u64> {
        self.0.load(SeqCst).checked_sub(1)
    }
    pub fn next(&self) -> u64 {
        self.0.load(SeqCst)
    }
    pub fn set_current(&self, block_n: Option<u64>) {
        self.0.store(block_n.map(|block_n| block_n + 1).unwrap_or(0), SeqCst)
    }
}

impl Clone for BlockNStatus {
    fn clone(&self) -> Self {
        Self(self.0.load(SeqCst).into())
    }
}

/// Counter of the latest block currently in the database.
/// We have multiple counters because the sync pipeline is split in sub-pipelines.
// TODO: move the L1 head block_n counter here too.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct ChainHead {
    // Individual pipeline progress.
    pub headers: BlockNStatus,
    pub state_diffs: BlockNStatus,
    pub classes: BlockNStatus,
    pub transactions: BlockNStatus,
    pub events: BlockNStatus,
    pub global_trie: BlockNStatus,

    /// Incremented by [`MadaraBackend::on_block`].
    pub full_block: BlockNStatus,
}

impl ChainHead {
    /// When None, the genesis block is not yet in the database.
    pub fn latest_full_block_n(&self) -> Option<u64> {
        self.full_block.current()
    }

    /// The next block to import.
    pub fn next_full_block(&self) -> u64 {
        self.latest_full_block_n().map(|n| n + 1).unwrap_or(0)
    }

    pub fn set_latest_full_block_n(&self, block_n: Option<u64>) {
        self.full_block.set_current(block_n);
    }

    pub(crate) fn load_from_db(db: &DB) -> Result<Self, MadaraStorageError> {
        let col = db.get_column(Column::BlockStorageMeta);
        if let Some(res) = db.get_pinned_cf(&col, ROW_HEAD_STATUS)? {
            return Ok(bincode::deserialize(res.as_ref())?);
        }
        Ok(Default::default())
    }
}

const ROW_HEAD_STATUS: &[u8] = b"head_status";

impl MadaraBackend {
    /// Chain tip status. In this context, the "head" or "tip" of the blockchain is the latest
    /// block in database.
    pub fn head_status(&self) -> &ChainHead {
        &self.head_status
    }
    pub fn load_head_status_from_db(&mut self) -> Result<(), MadaraStorageError> {
        self.head_status = ChainHead::load_from_db(&self.db)?;
        Ok(())
    }
    pub fn save_head_status_to_db(&self) -> Result<(), MadaraStorageError> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        self.db.put_cf_opt(&col, ROW_HEAD_STATUS, bincode::serialize(&self.head_status)?, &self.writeopts_no_wal)?;
        Ok(())
    }
}
