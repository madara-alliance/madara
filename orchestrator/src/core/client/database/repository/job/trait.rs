use async_trait::async_trait;
use uuid::Uuid;

use crate::core::client::database::error::DatabaseError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};

/// Repository for job CRUD operations and queries
///
/// This trait defines all job-related database operations.
/// Implementations are database-agnostic - the trait has no MongoDB types.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JobRepository: Send + Sync {
    // =========================================================================
    // CRUD Operations
    // =========================================================================

    /// Create a new job (fails if job with same type+internal_id exists)
    async fn create_job(&self, job: JobItem) -> Result<JobItem, DatabaseError>;

    /// Get job by UUID
    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>, DatabaseError>;

    /// Get job by internal ID and type
    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// Update a job (uses optimistic locking via version field)
    async fn update_job(&self, current_job: &JobItem, update: JobItemUpdates) -> Result<JobItem, DatabaseError>;

    // =========================================================================
    // Query Operations
    // =========================================================================

    /// Get the latest job of a specific type (by highest internal_id)
    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError>;

    /// Get the latest job of a specific type and status
    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// Get all jobs with a specific status
    async fn get_jobs_by_status(&self, status: JobStatus) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs filtered by types and statuses
    async fn get_jobs_by_types_and_statuses(
        &self,
        job_types: Vec<JobType>,
        statuses: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs of a specific type with multiple statuses
    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs after a specific internal ID
    async fn get_jobs_after_internal_id_by_job_type(
        &self,
        job_type: JobType,
        job_status: JobStatus,
        internal_id: String,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs between internal IDs (inclusive range)
    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        from_id: u64,
        to_id: u64,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get all jobs for a specific block number
    async fn get_jobs_by_block_number(&self, block_number: u64) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs that don't have a successor job of the specified type
    async fn get_jobs_without_successor(
        &self,
        job_a_type: JobType,
        job_a_status: JobStatus,
        job_b_type: JobType,
    ) -> Result<Vec<JobItem>, DatabaseError>;
}
