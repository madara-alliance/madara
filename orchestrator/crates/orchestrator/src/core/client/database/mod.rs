pub mod error;
pub mod mongodb;

use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use async_trait::async_trait;
pub use error::DatabaseError;
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

/// Trait defining database operations
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
    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>, DatabaseError>;
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

    // /// this need to been uncommendted once the session is introduced
    // ///
    // async fn insert<T>(&self, table: &str, document: &T) -> OrchestratorResult<String>
    // where
    //     T: Serialize + Send + Sync;
    //
    // async fn find_document<T, F>(&self, collection: &str, filter: &F) -> OrchestratorResult<Option<T>>
    // where
    //     T: DeserializeOwned + Send + Sync,
    //     F: Serialize + Send + Sync;
    //
    // async fn find_documents<T, F>(&self, collection: &str, filter: &F) -> OrchestratorResult<Vec<T>>
    // where
    //     T: DeserializeOwned + Send + Sync,
    //     F: Serialize + Send + Sync;
    //
    // async fn update_document<F, U>(&self, collection: &str, filter: &F, update: &U) -> OrchestratorResult<bool>
    // where
    //     F: Serialize + Send + Sync,
    //     U: Serialize + Send + Sync;
    //
    // async fn delete_document<F>(&self, collection: &str, filter: &F) -> OrchestratorResult<bool>
    // where
    //     F: Serialize + Send + Sync;
    //
    // async fn count_documents<F>(&self, collection: &str, filter: &F) -> OrchestratorResult<u64>
    // where
    //     F: Serialize + Send + Sync;
}
