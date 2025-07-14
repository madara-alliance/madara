pub mod error;
pub mod mongodb;

use crate::types::batch::{Batch, BatchStatus, BatchUpdates};
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

    /// get_latest_batch - Get the latest batch from DB. Returns `None` if the DB is empty
    async fn get_latest_batch(&self) -> Result<Option<Batch>, DatabaseError>;
    /// get_batches_by_index - Get all the batches with the given indexes
    async fn get_batches_by_indexes(&self, indexes: Vec<u64>) -> Result<Vec<Batch>, DatabaseError>;
    async fn update_batch_status_by_index(&self, index: u64, status: BatchStatus) -> Result<Batch, DatabaseError>;
    async fn update_batch(
        &self,
        filter: Document,
        update: Document,
        options: FindOneAndUpdateOptions,
        start: Instant,
        index: u64,
    ) -> Result<Batch, DatabaseError>;
    /// update_batch - Update the bath
    async fn update_or_create_batch(&self, batch: &Batch, update: &BatchUpdates) -> Result<Batch, DatabaseError>;
    /// create_batch - Create a new batch
    async fn create_batch(&self, batch: Batch) -> Result<Batch, DatabaseError>;
    /// get_batch_for_block - Returns the batch for a given block
    async fn get_batch_for_block(&self, block_number: u64) -> Result<Option<Batch>, DatabaseError>;
    /// get_batches_by_status - Get all the batches by that matches the given status
    async fn get_batches_by_status(&self, status: BatchStatus, limit: Option<i64>)
        -> Result<Vec<Batch>, DatabaseError>;
    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        gte: u64,
        lte: u64,
    ) -> Result<Vec<JobItem>, DatabaseError>;
}
