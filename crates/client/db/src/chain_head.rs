use crate::DatabaseExt;
use crate::{Column, MadaraBackend, MadaraStorageError};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

/// Counter of the latest block currently in the database.
/// We have multiple counters because the sync pipeline is split in sub-pipelines.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct ChainHead {
    headers: AtomicU64,
    state_diffs: AtomicU64,
    classes: AtomicU64,
    transactions: AtomicU64,
    l1_head: AtomicU64,
}

impl ChainHead {
    pub fn latest_full_block_n(&self) -> u64 {
        let state_diffs = self.state_diffs.load(SeqCst);
        let classes = self.classes.load(SeqCst);
        let transactions = self.transactions.load(SeqCst);
        state_diffs.min(classes).min(transactions)
    }

    pub fn latest_block_n_on_l1(&self) -> u64 {
        self.l1_head.load(SeqCst)
    }

    pub fn set_latest_header(&self, block_n: u64) {
        self.headers.store(block_n, SeqCst)
    }
    pub fn set_latest_state_diff(&self, block_n: u64) {
        self.state_diffs.store(block_n, SeqCst)
    }
    pub fn set_latest_class(&self, block_n: u64) {
        self.classes.store(block_n, SeqCst)
    }
    pub fn set_latest_transaction(&self, block_n: u64) {
        self.transactions.store(block_n, SeqCst)
    }
    pub fn set_l1_head(&self, block_n: u64) {
        self.l1_head.store(block_n, SeqCst)
    }
}

const ROW_HEAD_STATUS: &[u8] = b"head_status";

impl MadaraBackend {
    pub fn head_status(&self) -> &ChainHead {
        &self.head_status
    }
    pub fn load_head_status_from_db(&mut self) -> Result<(), MadaraStorageError> {
        let col = self.db.get_column(Column::BlockStorageMeta);
        self.head_status = Default::default();
        if let Some(res) = self.db.get_pinned_cf(&col, ROW_HEAD_STATUS)? {
            self.head_status = bincode::deserialize(res.as_ref())?;
        }
        Ok(())
    }
}
