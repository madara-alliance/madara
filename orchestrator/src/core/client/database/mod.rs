pub mod constant;
pub mod error;
pub mod mongodb;

use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatch, SnosBatchStatus, SnosBatchUpdates,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use ::mongodb::bson::Document;
use ::mongodb::options::FindOneAndUpdateOptions;
use async_trait::async_trait;
pub use error::DatabaseError;
use std::time::Instant;

/// Trait defining database operations for the orchestrator
///
/// This trait provides a comprehensive interface for all database operations
/// needed by the orchestrator, including job management and batch management.
///
/// # Batch Management
/// The database manages two types of batches with a hierarchical relationship:
/// - **Aggregator Batches**: High-level batches that manage the proving pipeline
/// - **SNOS Batches**: Lower-level batches contained within aggregator batches
///
/// # Relationship Rules
/// - Each SNOS batch belongs to exactly one aggregator batch
/// - Multiple SNOS batches can exist within one aggregator batch  
/// - When an aggregator batch closes, all its SNOS batches are closed
/// - SNOS batches can close independently within an open aggregator batch
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait DatabaseClient: Send + Sync {
    /// Switch to a different database
    ///
    /// # Arguments
    /// * `database_name` - Name of the database to switch to
    async fn switch_database(&mut self, database_name: &str) -> Result<(), DatabaseError>;

    /// Disconnect from the database
    async fn disconnect(&self) -> Result<(), DatabaseError>;

    // ================================================================================
    // Job Management Methods
    // ================================================================================

    /// Create a new job in the database
    ///
    /// # Arguments
    /// * `job` - The job item to create
    async fn create_job(&self, job: JobItem) -> Result<JobItem, DatabaseError>;

    /// Get a job by its ID
    ///
    /// # Arguments
    /// * `id` - The UUID of the job to retrieve
    async fn get_job_by_id(&self, id: uuid::Uuid) -> Result<Option<JobItem>, DatabaseError>;

    /// Get a job by its internal ID and type
    ///
    /// # Arguments
    /// * `internal_id` - The internal identifier of the job
    /// * `job_type` - The type of job to search for
    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// Update a job in the database
    ///
    /// # Arguments
    /// * `current_job` - The current state of the job
    /// * `update` - The updates to apply
    async fn update_job(&self, current_job: &JobItem, update: JobItemUpdates) -> Result<JobItem, DatabaseError>;

    /// Get the latest job of a specific type
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError>;

    /// Get jobs without a successor
    ///
    /// # Arguments
    /// * `job_a_type` - Type of the first job
    /// * `job_a_status` - Status of the first job
    /// * `job_b_type` - Type of the successor job to check for
    async fn get_jobs_without_successor(
        &self,
        job_a_type: JobType,
        job_a_status: JobStatus,
        job_b_type: JobType,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get the latest job of a specific type and status
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    /// * `job_status` - The status of job to search for
    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// Get jobs after a specific internal id by job type
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    /// * `job_status` - The status of jobs to search for
    /// * `internal_id` - The internal ID threshold (exclusive)
    async fn get_jobs_after_internal_id_by_job_type(
        &self,
        job_type: JobType,
        job_status: JobStatus,
        internal_id: String,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get all jobs by types and statuses
    ///
    /// # Arguments
    /// * `job_type` - Vector of job types to search for
    /// * `status` - Vector of statuses to search for
    /// * `limit` - Optional limit on number of results
    async fn get_jobs_by_types_and_statuses(
        &self,
        job_type: Vec<JobType>,
        status: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs between internal IDs for a specific type and status
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    /// * `status` - The status of jobs to search for
    /// * `gte` - Greater than or equal to this internal ID
    /// * `lte` - Less than or equal to this internal ID
    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        gte: u64,
        lte: u64,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs by their type and statuses
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    /// * `job_statuses` - Vector of statuses to match
    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        job_statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs by their type and statuses with internal_id constraint
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    /// * `job_status` - Vector of statuses to match
    /// * `max_internal_id` - Maximum internal_id value (inclusive) - compared numerically
    async fn get_jobs_excluding_statuses_up_to_internal_id(
        &self,
        job_type: JobType,
        job_status: Vec<JobStatus>,
        max_internal_id: u64,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get all jobs for a specific block number
    ///
    /// # Arguments
    /// * `block_number` - The block number to search for
    async fn get_jobs_by_block_number(&self, block_number: u64) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs stuck in LockedForProcessing status beyond timeout for specific job type
    ///
    /// # Arguments
    /// * `job_type` - The type of job to search for
    /// * `timeout_seconds` - Timeout threshold in seconds
    async fn get_orphaned_jobs(&self, job_type: &JobType, timeout_seconds: u64) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get all jobs by status
    async fn get_jobs_by_status(&self, status: JobStatus) -> Result<Vec<JobItem>, DatabaseError>;

    // ================================================================================
    // SNOS Batch Management Methods
    // ================================================================================

    /// Get the latest SNOS batch from the database
    ///
    /// Returns the SNOS batch with the highest `snos_batch_id`, or `None` if no batches exist.
    async fn get_latest_snos_batch(&self) -> Result<Option<SnosBatch>, DatabaseError>;

    /// Get SNOS batches by their sequential IDs
    ///
    /// # Arguments
    /// * `indexes` - Vector of SNOS batch IDs to retrieve
    async fn get_snos_batches_by_indices(&self, indexes: Vec<u64>) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Update SNOS batch status by its sequential ID
    ///
    /// # Arguments
    /// * `index` - The SNOS batch ID to update
    /// * `status` - The new status to set
    async fn update_snos_batch_status_by_index(
        &self,
        index: u64,
        status: SnosBatchStatus,
    ) -> Result<SnosBatch, DatabaseError>;

    /// Get SNOS batches by status
    ///
    /// # Arguments
    /// * `status` - The status to filter by
    /// * `limit` - Optional limit on number of results
    async fn get_snos_batches_by_status(
        &self,
        status: SnosBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get SNOS batches that don't have corresponding SNOS jobs
    ///
    /// This method finds all SNOS batches that don't have a matching SNOS job
    /// based on the relationship where snos_batch_id equals job internal_id.
    /// It checks for SNOS jobs in ANY status.
    ///
    /// # Arguments
    /// * `snos_batch_status` - Status of SNOS batches to check (typically Closed)
    ///
    /// # Returns
    /// Vector of SNOS batches that don't have corresponding SNOS jobs (in any status)
    async fn get_snos_batches_without_jobs(
        &self,
        snos_batch_status: SnosBatchStatus,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Update or create a SNOS batch
    ///
    /// # Arguments
    /// * `batch` - The SNOS batch to update/create
    /// * `update` - The updates to apply
    async fn update_or_create_snos_batch(
        &self,
        batch: &SnosBatch,
        update: &SnosBatchUpdates,
    ) -> Result<SnosBatch, DatabaseError>;

    /// Create a new SNOS batch
    ///
    /// # Arguments
    /// * `batch` - The SNOS batch to create
    async fn create_snos_batch(&self, batch: SnosBatch) -> Result<SnosBatch, DatabaseError>;

    /// Update a SNOS batch with custom filter and update documents
    ///
    /// # Arguments
    /// * `filter` - MongoDB filter document
    /// * `update` - MongoDB update document
    /// * `options` - Update options
    /// * `start` - Start time for metrics
    /// * `index` - SNOS batch ID for logging
    async fn update_snos_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<SnosBatch, DatabaseError>;

    // ================================================================================
    // Aggregator Batch Management Methods
    // ================================================================================

    /// Get the latest aggregator batch from the database
    ///
    /// Returns the aggregator batch with the highest `index`, or `None` if no batches exist.
    async fn get_latest_aggregator_batch(&self) -> Result<Option<AggregatorBatch>, DatabaseError>;

    /// Get aggregator batches by their indexes
    ///
    /// # Arguments
    /// * `indexes` - Vector of aggregator batch indexes to retrieve
    async fn get_aggregator_batches_by_indexes(&self, indexes: Vec<u64>)
        -> Result<Vec<AggregatorBatch>, DatabaseError>;

    /// Update aggregator batch status by its index
    ///
    /// # Arguments
    /// * `index` - The aggregator batch index to update
    /// * `status` - The new status to set
    async fn update_aggregator_batch_status_by_index(
        &self,
        index: u64,
        status: AggregatorBatchStatus,
    ) -> Result<AggregatorBatch, DatabaseError>;

    /// Update an aggregator batch with custom filter and update documents
    ///
    /// # Arguments
    /// * `filter` - MongoDB filter document
    /// * `update` - MongoDB update document
    /// * `options` - Update options
    /// * `start` - Start time for metrics
    /// * `index` - Aggregator batch index for logging
    async fn update_aggregator_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<AggregatorBatch, DatabaseError>;

    /// Update or create an aggregator batch
    ///
    /// # Arguments
    /// * `batch` - The aggregator batch to update/create
    /// * `update` - The updates to apply
    async fn update_or_create_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        update: &AggregatorBatchUpdates,
    ) -> Result<AggregatorBatch, DatabaseError>;

    /// Create a new aggregator batch
    ///
    /// # Arguments
    /// * `batch` - The aggregator batch to create
    async fn create_aggregator_batch(&self, batch: AggregatorBatch) -> Result<AggregatorBatch, DatabaseError>;

    /// Get the aggregator batch that contains a specific block
    ///
    /// # Arguments
    /// * `block_number` - The block number to search for
    async fn get_aggregator_batch_for_block(&self, block_number: u64)
        -> Result<Option<AggregatorBatch>, DatabaseError>;

    /// Get aggregator batches by status
    ///
    /// # Arguments
    /// * `status` - The status to filter by
    /// * `limit` - Optional limit on number of results
    async fn get_aggregator_batches_by_status(
        &self,
        status: AggregatorBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError>;

    // ================================================================================
    // Batch Relationship Management Methods
    // ================================================================================

    /// Get all SNOS batches belonging to a specific aggregator batch
    ///
    /// # Arguments
    /// * `aggregator_index` - The index of the aggregator batch
    ///
    /// # Returns
    /// Vector of all SNOS batches (any status) that belong to the specified aggregator batch
    async fn get_snos_batches_by_aggregator_index(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get open SNOS batches for a specific aggregator batch
    ///
    /// # Arguments
    /// * `aggregator_index` - The index of the aggregator batch
    ///
    /// # Returns
    /// Vector of SNOS batches with status `Open` that belong to the specified aggregator batch
    async fn get_open_snos_batches_by_aggregator_index(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// Get the next available SNOS batch ID
    ///
    /// This method finds the highest existing `snos_batch_id` and returns the next sequential number.
    /// If no SNOS batches exist, returns 1.
    ///
    /// # Returns
    /// The next available SNOS batch ID for creating new batches
    async fn get_next_snos_batch_id(&self) -> Result<u64, DatabaseError>;

    /// Close all SNOS batches for a specific aggregator batch
    ///
    /// This method is used when an aggregator batch closes to ensure all its
    /// contained SNOS batches are also closed, maintaining the hierarchical
    /// relationship constraint.
    ///
    /// # Arguments
    /// * `aggregator_index` - The index of the aggregator batch whose SNOS batches should be closed
    ///
    /// # Returns
    /// Vector of all SNOS batches that were closed by this operation
    async fn close_all_snos_batches_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    // ================================================================================
    // Health Check Methods
    // ================================================================================

    /// Perform a health check on the database connection
    ///
    /// This method verifies that the database is accessible and operational.
    /// It should perform a lightweight operation that validates connectivity
    /// and basic functionality without impacting performance.
    ///
    /// # Returns
    /// * `Ok(())` - If the database is healthy and accessible
    /// * `Err(DatabaseError)` - If the health check fails
    async fn health_check(&self) -> Result<(), DatabaseError>;
}
