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
    async fn test_update_sync_status() {
        let provider = SyncStatusProvider::new();
        let new_status = SyncStatus {
            starting_block_num: 10,
            starting_block_hash: Felt::from_hex("0x1").unwrap(),
            current_block_num: 20,
            current_block_hash: Felt::from_hex("0x2").unwrap(),
            highest_block_num: 30,
            highest_block_hash: Felt::from_hex("0x3").unwrap(),
        };

        provider.update_sync_status(new_status.clone()).await;
        let status = provider.get_sync_status().await;

        assert_eq!(status, new_status);
    }

    #[tokio::test]
    async fn test_set_starting_block() {
        let provider = SyncStatusProvider::new();
        let block_num = 5;
        let block_hash = Felt::from_hex("0x5").unwrap();

        provider.set_starting_block(block_num, block_hash).await;
        let status = provider.get_sync_status().await;

        assert_eq!(status.starting_block_num, block_num);
        assert_eq!(status.starting_block_hash, block_hash);
    }

    #[tokio::test]
    async fn test_set_current_block() {
        let provider = SyncStatusProvider::new();
        let block_num = 15;
        let block_hash = Felt::from_hex("0xf").unwrap();

        provider.set_current_block(block_num, block_hash).await;
        let status = provider.get_sync_status().await;

        assert_eq!(status.current_block_num, block_num);
        assert_eq!(status.current_block_hash, block_hash);
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

        // Test individual number setters
        provider.set_starting_block_num(5).await;
        provider.set_current_block_num(15).await;
        provider.set_highest_block_num(25).await;

        // Test individual hash setters
        let starting_hash = Felt::from_hex("0x1").unwrap();
        let current_hash = Felt::from_hex("0x2").unwrap();
        let highest_hash = Felt::from_hex("0x3").unwrap();

        provider.set_starting_block_hash(starting_hash).await;
        provider.set_current_block_hash(current_hash).await;
        provider.set_highest_block_hash(highest_hash).await;

        let status = provider.get_sync_status().await;

        // Verify all fields were set correctly
        assert_eq!(status.starting_block_num, 5);
        assert_eq!(status.current_block_num, 15);
        assert_eq!(status.highest_block_num, 25);
        assert_eq!(status.starting_block_hash, starting_hash);
        assert_eq!(status.current_block_hash, current_hash);
        assert_eq!(status.highest_block_hash, highest_hash);
    }

    #[tokio::test]
    async fn test_default_implementation() {
        let provider = SyncStatusProvider::default();
        let status = provider.get_sync_status().await;

        assert_eq!(status.starting_block_num, 0);
        assert_eq!(status.current_block_num, 0);
        assert_eq!(status.highest_block_num, 0);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let provider = Arc::new(SyncStatusProvider::new());
        let provider_clone1 = Arc::clone(&provider);
        let provider_clone2 = Arc::clone(&provider);

        // Task 1: Update starting block
        let task1 = task::spawn(async move {
            provider_clone1.set_starting_block(10, Felt::from_hex("0xa").unwrap()).await;
        });

        // Task 2: Update current block
        let task2 = task::spawn(async move {
            provider_clone2.set_current_block(20, Felt::from_hex("0x14").unwrap()).await;
        });

        // Wait for both tasks to complete
        let _ = tokio::join!(task1, task2);

        // Verify updates were applied correctly
        let status = provider.get_sync_status().await;
        assert_eq!(status.starting_block_num, 10);
        assert_eq!(status.starting_block_hash, Felt::from_hex("0xa").unwrap());
        assert_eq!(status.current_block_num, 20);
        assert_eq!(status.current_block_hash, Felt::from_hex("0x14").unwrap());
    }

    #[tokio::test]
    async fn test_sync_status_clone() {
        let provider = SyncStatusProvider::new();

        // Set some values
        provider.set_starting_block(5, Felt::from_hex("0x5").unwrap()).await;
        provider.set_current_block(10, Felt::from_hex("0xa").unwrap()).await;
        provider.set_highest_block(15, Felt::from_hex("0xf").unwrap()).await;

        // Get and clone the status
        let status1 = provider.get_sync_status().await;
        let status2 = status1.clone();

        // Verify clone is identical
        assert_eq!(status1, status2);

        // Modify the provider after cloning
        provider.set_current_block(20, Felt::from_hex("0x14").unwrap()).await;

        // Verify the clone remains unchanged
        assert_eq!(status2.current_block_num, 10);
        assert_eq!(status2.current_block_hash, Felt::from_hex("0xa").unwrap());

        // Verify the provider was updated
        let updated_status = provider.get_sync_status().await;
        assert_eq!(updated_status.current_block_num, 20);
        assert_eq!(updated_status.current_block_hash, Felt::from_hex("0x14").unwrap());
    }

    #[tokio::test]
    async fn test_edge_cases() {
        let provider = SyncStatusProvider::new();

        // Test with maximum u64 value
        let max_block_num = u64::MAX;
        provider.set_highest_block_num(max_block_num).await;

        let status = provider.get_sync_status().await;
        assert_eq!(status.highest_block_num, max_block_num);

        // Test with zero hash
        let zero_hash = Felt::from_hex("0x0").unwrap();
        provider.set_current_block_hash(zero_hash).await;

        let status = provider.get_sync_status().await;
        assert_eq!(status.current_block_hash, zero_hash);
    }
}
