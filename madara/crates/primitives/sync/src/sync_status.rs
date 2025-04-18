use mp_rpc::{BlockHash, BlockNumber, SyncStatus};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Provider for synchronization status
#[derive(Clone)]
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
                starting_block_num: BlockNumber::default(),
                starting_block_hash: BlockHash::default(),
                current_block_num: BlockNumber::default(),
                current_block_hash: BlockHash::default(),
                highest_block_num: BlockNumber::default(),
                highest_block_hash: BlockHash::default(),
            })),
        }
    }

    /// Get the current sync status
    pub async fn get_sync_status(&self) -> SyncStatus {
        self.sync_status.read().await.clone()
    }

    /// Set both number and hash for highest block
    pub async fn set_highest_block(&self, block_num: BlockNumber, block_hash: BlockHash) {
        let mut status = self.sync_status.write().await;
        status.highest_block_num = block_num;
        status.highest_block_hash = block_hash;
    }
}

#[cfg(test)]
mod sync_status_tests {
    use super::*;
    use starknet_types_core::felt::Felt;
    use tokio::task;

    #[tokio::test]
    async fn test_new_default_values() {
        let provider = SyncStatusProvider::new();
        let status = provider.get_sync_status().await;

        assert_eq!(status.starting_block_num, 0);
        assert_eq!(status.starting_block_hash, BlockHash::default());
        assert_eq!(status.current_block_num, 0);
        assert_eq!(status.current_block_hash, BlockHash::default());
        assert_eq!(status.highest_block_num, 0);
        assert_eq!(status.highest_block_hash, BlockHash::default());
    }

    #[tokio::test]
    async fn test_set_highest_block() {
        let provider = SyncStatusProvider::new();
        let block_num = 25;
        let block_hash = Felt::from_hex("0x19").unwrap();

        provider.set_highest_block(block_num, block_hash).await;
        let status = provider.get_sync_status().await;

        assert_eq!(status.highest_block_num, block_num);
        assert_eq!(status.highest_block_hash, block_hash);
    }

    #[tokio::test]
    async fn test_set_individual_fields() {
        let provider = SyncStatusProvider::new();

        let highest_hash = Felt::from_hex("0x3").unwrap();
        provider.set_highest_block(25, highest_hash).await;

        let status = provider.get_sync_status().await;

        // Verify all fields were set correctly
        assert_eq!(status.highest_block_num, 25);
        assert_eq!(status.highest_block_hash, highest_hash);
    }

    #[tokio::test]
    async fn test_default_implementation() {
        let provider = SyncStatusProvider::default();
        let status = provider.get_sync_status().await;

        assert_eq!(status.starting_block_num, 0);
        assert_eq!(status.highest_block_num, 0);
        assert_eq!(status.current_block_num, 0);
    }

    #[tokio::test]
    async fn test_edge_cases() {
        let provider = SyncStatusProvider::new();

        // Test with maximum u64 value and zero hash
        let max_block_num = u64::MAX;
        let zero_hash = Felt::from_hex("0x0").unwrap();

        provider.set_highest_block(max_block_num, zero_hash).await;
        let status = provider.get_sync_status().await;

        assert_eq!(status.highest_block_hash, zero_hash);
        assert_eq!(status.highest_block_num, max_block_num);
    }
}
