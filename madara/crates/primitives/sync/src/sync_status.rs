use mp_rpc::{BlockHash, BlockNumber, SyncStatus};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Provider for synchronization status
pub struct SyncStatusProvider {
    sync_status: Arc<RwLock<SyncStatus>>,
}

impl Default for SyncStatusProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncStatusProvider {
    /// Create a new SyncStatusProvider with default values
    pub fn new() -> Self {
        Self {
            sync_status: Arc::new(RwLock::new(SyncStatus {
                starting_block_num: 0,
                starting_block_hash: BlockHash::default(),
                current_block_num: 0,
                current_block_hash: BlockHash::default(),
                highest_block_num: 0,
                highest_block_hash: BlockHash::default(),
            })),
        }
    }

    /// Update the entire sync status
    pub async fn update_sync_status(&self, new_sync_status: SyncStatus) {
        let mut status = self.sync_status.write().await;
        *status = new_sync_status;
    }

    /// Get the current sync status
    pub async fn get_sync_status(&self) -> SyncStatus {
        self.sync_status.read().await.clone()
    }

    /// Set both number and hash for starting block
    pub async fn set_starting_block(&self, block_num: BlockNumber, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.starting_block_num = block_num;
        status.starting_block_hash = block_hash;
    }

    /// Set both number and hash for current block
    pub async fn set_current_block(&self, block_num: BlockNumber, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.current_block_num = block_num;
        status.current_block_hash = block_hash;
    }

    /// Set both number and hash for highest block
    pub async fn set_highest_block(&self, block_num: BlockNumber, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.highest_block_num = block_num;
        status.highest_block_hash = block_hash;
    }

    /// Set starting block number
    pub async fn set_starting_block_num(&self, block_num: BlockNumber) {
        let mut status = self.sync_status.write().await;
        status.starting_block_num = block_num;
    }

    /// Set current block number
    pub async fn set_current_block_num(&self, block_num: BlockNumber) {
        let mut status = self.sync_status.write().await;
        status.current_block_num = block_num;
    }

    /// Set highest block number
    pub async fn set_highest_block_num(&self, block_num: BlockNumber) {
        let mut status = self.sync_status.write().await;
        status.highest_block_num = block_num;
    }

    /// Set starting block hash
    pub async fn set_starting_block_hash(&self, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.starting_block_hash = block_hash;
    }

    /// Set current block hash
    pub async fn set_current_block_hash(&self, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.current_block_hash = block_hash;
    }

    /// Set highest block hash
    pub async fn set_highest_block_hash(&self, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.highest_block_hash = block_hash;
    }
}
