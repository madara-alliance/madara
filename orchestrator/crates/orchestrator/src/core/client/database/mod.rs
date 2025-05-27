pub mod error;
pub mod mongodb;

use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use async_trait::async_trait;
pub use error::DatabaseError;

/// Trait defining database operations
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait DatabaseClient: Send + Sync {
    /// switch_database - switch to a different database
    async fn switch_database(&mut self, database_name: &str) -> Result<(), DatabaseError>;

    /// disconnect - Disconnect from the database
    async fn disconnect(&self) -> Result<(), DatabaseError>;

    /// ENHANCEMENT: following method are supposed to be generic, but we need to figure out how to serialize them
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
    /// get_jobs_by_statuses - Get jobs by their statuses
    async fn get_jobs_by_statuses(
        &self,
        status: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// get_jobs_by_type_and_statuses - Get jobs by their type and statuses
    /// This method is used to get jobs by their type and statuses.
    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        job_statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError>;
}
