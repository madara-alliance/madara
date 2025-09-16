pub mod constant;
pub mod error;
pub mod mongodb;

use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, Batch, BatchType, SnosBatch, SnosBatchStatus,
    SnosBatchUpdates,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use ::mongodb::bson::Document;
use ::mongodb::options::FindOneAndUpdateOptions;
use async_trait::async_trait;
pub use error::DatabaseError;
use std::time::Instant;

/// Trait defining database operations
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait DatabaseClient: Send + Sync {
    /// switch_database - switch to a different database
    async fn switch_database(&mut self, database_name: &str) -> Result<(), DatabaseError>;

    /// disconnect - Disconnect from the database
    async fn disconnect(&self) -> Result<(), DatabaseError>;

    /// ENHANCEMENT: the following method is supposed to be generic, but we need to figure out how to serialize them
    /// create_job - Create a new job in the database
    async fn create_job(&self, job: JobItem) -> Result<JobItem, DatabaseError>;
    /// get_job_by_id - Get a job by its ID
    async fn get_job_by_id(&self, id: uuid::Uuid) -> Result<Option<JobItem>, DatabaseError>;
    /// get_job_by_internal_id_and_type - Get a job by its internal ID and type
    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError>;
    /// update_job - Update a job in the database
    async fn update_job(&self, current_job: &JobItem, update: JobItemUpdates) -> Result<JobItem, DatabaseError>;
    /// get_latest_job_by_type - Get the latest job of a specific type
    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError>;
    /// get_jobs_without_successor - Get jobs without a successor
    async fn get_jobs_without_successor(
        &self,
        job_a_type: JobType,
        job_a_status: JobStatus,
        job_b_type: JobType,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// get_latest_job_by_type_and_status - Get the latest job of a specific type and status
    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// get_jobs_after_internal_id_by_job_type - Get jobs after a specific internal id by job type
    async fn get_jobs_after_internal_id_by_job_type(
        &self,
        job_type: JobType,
        job_status: JobStatus,
        internal_id: String,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// get_jobs_by_statuses -  Get all the jobs by types and status
    async fn get_jobs_by_types_and_statuses(
        &self,
        job_type: Vec<JobType>,
        status: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// get_missing_jobs_by_type_and_caps - Get all the missed jobs by type and block number limits
    async fn get_missing_block_numbers_by_type_and_caps(
        &self,
        job_type: JobType,
        lower_cap: u64,
        upper_cap: u64,
        limit: Option<i64>,
    ) -> Result<Vec<u64>, DatabaseError>;

    /// get_latest_batch - Get the latest batch from DB based on batch type. Returns `None` if the DB is empty
    async fn get_latest_batch(&self, batch_type: BatchType) -> Result<Option<Batch>, DatabaseError>;

    /// get_latest_snos_batch - Get the latest SNOS batch from DB. Returns `None` if the DB is empty
    async fn get_latest_snos_batch(&self) -> Result<Option<SnosBatch>, DatabaseError>;
    /// get_snos_batch_by_index - Get the snos batch by indexk
    async fn get_snos_batches_by_indices(&self, indexes: Vec<u64>) -> Result<Vec<SnosBatch>, DatabaseError>;
    /// update_snos_batch_status_by_index - Update the snos batch status by index
    async fn update_snos_batch_status_by_index(
        &self,
        index: u64,
        status: SnosBatchStatus,
    ) -> Result<SnosBatch, DatabaseError>;
    /// get_latest_batch - Get the latest batch from DB. Returns `None` if the DB is empty
    async fn get_latest_aggregator_batch(&self) -> Result<Option<AggregatorBatch>, DatabaseError>;
    /// get_batches_by_index - Get all the batches with the given indexes
    async fn get_aggregator_batches_by_indexes(&self, indexes: Vec<u64>)
        -> Result<Vec<AggregatorBatch>, DatabaseError>;
    async fn update_aggregator_batch_status_by_index(
        &self,
        index: u64,
        status: AggregatorBatchStatus,
    ) -> Result<AggregatorBatch, DatabaseError>;
    async fn update_aggregator_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<AggregatorBatch, DatabaseError>;

    async fn update_snos_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<SnosBatch, DatabaseError>;
    /// update_batch - Update the bath
    async fn update_or_create_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        update: &AggregatorBatchUpdates,
    ) -> Result<AggregatorBatch, DatabaseError>;
    /// create_batch - Create a new batch
    async fn create_aggregator_batch(&self, batch: AggregatorBatch) -> Result<AggregatorBatch, DatabaseError>;
    /// get_batch_for_block - Returns the batch for a given block
    async fn get_aggregator_batch_for_block(&self, block_number: u64)
        -> Result<Option<AggregatorBatch>, DatabaseError>;
    /// get_batches_by_status - Get all the batches by that matches the given status
    async fn get_aggregator_batches_by_status(
        &self,
        status: AggregatorBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError>;

    /// get_snos_batches_by_status - Get all the snos batches by that matches the given status
    async fn get_snos_batches_by_status(
        &self,
        status: SnosBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<SnosBatch>, DatabaseError>;

    /// update_or_create_snos_batch - Update the snos batch
    async fn update_or_create_snos_batch(
        &self,
        batch: &SnosBatch,
        update: &SnosBatchUpdates,
    ) -> Result<SnosBatch, DatabaseError>;
    /// create_snos_batch - Create a new snos batch
    async fn create_snos_batch(&self, batch: SnosBatch) -> Result<SnosBatch, DatabaseError>;

    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        gte: u64,
        lte: u64,
    ) -> Result<Vec<JobItem>, DatabaseError>;
    /// get_jobs_by_type_and_statuses - Get jobs by their type and statuses
    /// This method is used to get jobs by their type and statuses.
    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        job_statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError>;
    /// get_jobs_by_block_number - Get all jobs for a specific block number
    async fn get_jobs_by_block_number(&self, block_number: u64) -> Result<Vec<JobItem>, DatabaseError>;
    /// get_orphaned_jobs - Get jobs stuck in LockedForProcessing status beyond timeout for specific job type
    async fn get_orphaned_jobs(&self, job_type: &JobType, timeout_seconds: u64) -> Result<Vec<JobItem>, DatabaseError>;
}
