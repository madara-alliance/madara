use async_trait::async_trait;

use crate::core::client::database::error::DatabaseError;
use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatch, SnosBatchStatus, SnosBatchUpdates,
};

/// Repository for SNOS and Aggregator batch operations
///
/// This trait handles both SNOS and Aggregator batches since they have
/// a hierarchical relationship (SNOS batches belong to Aggregator batches).
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait BatchRepository: Send + Sync {
    // =========================================================================
    // SNOS Batch Operations
    // =========================================================================

    /// Create a new SNOS batch
    async fn create_snos_batch(&self, batch: SnosBatch) -> Result<SnosBatch, DatabaseError>;

    /// Get the latest SNOS batch (highest index)
    async fn get_latest_snos_batch(&self) -> Result<Option<SnosBatch>, DatabaseError>;

    /// Get SNOS batches by their indices
    async fn get_snos_batches_by_indices(&self, indices: Vec<u64>) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get SNOS batches by status
    async fn get_snos_batches_by_status(
        &self,
        status: SnosBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get SNOS batches that don't have corresponding SNOS jobs
    async fn get_snos_batches_without_jobs(&self, status: SnosBatchStatus) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Update SNOS batch status by index
    async fn update_snos_batch_status(&self, index: u64, status: SnosBatchStatus) -> Result<SnosBatch, DatabaseError>;

    /// Update or create a SNOS batch
    async fn upsert_snos_batch(&self, batch: &SnosBatch, update: &SnosBatchUpdates)
        -> Result<SnosBatch, DatabaseError>;

    /// Get all SNOS batches belonging to an aggregator batch
    async fn get_snos_batches_by_aggregator(&self, aggregator_index: u64) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get open SNOS batches for an aggregator batch
    async fn get_open_snos_batches_by_aggregator(&self, aggregator_index: u64)
        -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get the first SNOS batch in an aggregator batch
    async fn get_first_snos_batch_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Option<SnosBatch>, DatabaseError>;

    /// Count SNOS batches in an aggregator batch
    async fn count_snos_batches_by_aggregator(&self, aggregator_index: u64) -> Result<u64, DatabaseError>;

    /// Get the next available SNOS batch ID
    async fn get_next_snos_batch_id(&self) -> Result<u64, DatabaseError>;

    /// Close all SNOS batches for an aggregator batch
    async fn close_snos_batches_for_aggregator(&self, aggregator_index: u64) -> Result<Vec<SnosBatch>, DatabaseError>;

    // =========================================================================
    // Aggregator Batch Operations
    // =========================================================================

    /// Create a new aggregator batch
    async fn create_aggregator_batch(&self, batch: AggregatorBatch) -> Result<AggregatorBatch, DatabaseError>;

    /// Get the latest aggregator batch (highest index)
    async fn get_latest_aggregator_batch(&self) -> Result<Option<AggregatorBatch>, DatabaseError>;

    /// Get aggregator batches by their indices
    async fn get_aggregator_batches_by_indices(&self, indices: Vec<u64>)
        -> Result<Vec<AggregatorBatch>, DatabaseError>;

    /// Get aggregator batches by status
    async fn get_aggregator_batches_by_status(
        &self,
        status: AggregatorBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError>;

    /// Get the aggregator batch containing a specific block
    async fn get_aggregator_batch_for_block(&self, block_number: u64)
        -> Result<Option<AggregatorBatch>, DatabaseError>;

    /// Update aggregator batch status by index
    async fn update_aggregator_batch_status(
        &self,
        index: u64,
        status: AggregatorBatchStatus,
    ) -> Result<AggregatorBatch, DatabaseError>;

    /// Update or create an aggregator batch
    async fn upsert_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        update: &AggregatorBatchUpdates,
    ) -> Result<AggregatorBatch, DatabaseError>;
}
