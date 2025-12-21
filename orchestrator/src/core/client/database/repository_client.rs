//! DatabaseClient implementation using the repository pattern
//!
//! This client implements the DatabaseClient trait by delegating to the
//! underlying repositories (JobRepository, BatchRepository, WorkerRepository).
//! It serves as an adapter layer that maintains the existing DatabaseClient
//! interface while using the new repository-based architecture internally.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::error::DatabaseError;
use super::repository::{BatchRepository, JobRepository, WorkerRepository};
use super::DatabaseClient;
use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatch, SnosBatchStatus, SnosBatchUpdates,
};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use mongodb::bson::Document;
use mongodb::options::FindOneAndUpdateOptions;

/// DatabaseClient implementation that uses repositories internally
pub struct RepositoryDatabaseClient {
    job_repo: Arc<dyn JobRepository>,
    batch_repo: Arc<dyn BatchRepository>,
    worker_repo: Arc<dyn WorkerRepository>,
}

impl RepositoryDatabaseClient {
    pub fn new(
        job_repo: Arc<dyn JobRepository>,
        batch_repo: Arc<dyn BatchRepository>,
        worker_repo: Arc<dyn WorkerRepository>,
    ) -> Self {
        Self { job_repo, batch_repo, worker_repo }
    }
}

#[async_trait]
impl DatabaseClient for RepositoryDatabaseClient {
    // Note: switch_database and disconnect are not supported in the repository model
    // These methods are database-admin level operations that don't fit the repository pattern
    async fn switch_database(&mut self, _database_name: &str) -> Result<(), DatabaseError> {
        Err(DatabaseError::UpdateFailed("switch_database not supported in repository-based client".to_string()))
    }

    async fn disconnect(&self) -> Result<(), DatabaseError> {
        // Repositories manage their own connections, no explicit disconnect needed
        Ok(())
    }

    // ================================================================================
    // Job Management Methods - delegate to JobRepository
    // ================================================================================

    async fn create_job(&self, job: JobItem) -> Result<JobItem, DatabaseError> {
        self.job_repo.create_job(job).await
    }

    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>, DatabaseError> {
        self.job_repo.get_job_by_id(id).await
    }

    async fn get_job_by_internal_id_and_type(
        &self,
        internal_id: &str,
        job_type: &JobType,
    ) -> Result<Option<JobItem>, DatabaseError> {
        self.job_repo.get_job_by_internal_id_and_type(internal_id, job_type).await
    }

    async fn update_job(&self, current_job: &JobItem, update: JobItemUpdates) -> Result<JobItem, DatabaseError> {
        self.job_repo.update_job(current_job, update).await
    }

    async fn get_latest_job_by_type(&self, job_type: JobType) -> Result<Option<JobItem>, DatabaseError> {
        self.job_repo.get_latest_job_by_type(job_type).await
    }

    async fn get_jobs_without_successor(
        &self,
        job_a_type: JobType,
        job_a_status: JobStatus,
        job_b_type: JobType,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_without_successor(job_a_type, job_a_status, job_b_type).await
    }

    async fn get_latest_job_by_type_and_status(
        &self,
        job_type: JobType,
        job_status: JobStatus,
    ) -> Result<Option<JobItem>, DatabaseError> {
        self.job_repo.get_latest_job_by_type_and_status(job_type, job_status).await
    }

    async fn get_jobs_after_internal_id_by_job_type(
        &self,
        job_type: JobType,
        job_status: JobStatus,
        internal_id: String,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_after_internal_id_by_job_type(job_type, job_status, internal_id).await
    }

    async fn get_jobs_by_types_and_statuses(
        &self,
        job_type: Vec<JobType>,
        status: Vec<JobStatus>,
        limit: Option<i64>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_by_types_and_statuses(job_type, status, limit).await
    }

    async fn get_jobs_between_internal_ids(
        &self,
        job_type: JobType,
        status: JobStatus,
        gte: u64,
        lte: u64,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_between_internal_ids(job_type, status, gte, lte).await
    }

    async fn get_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        job_statuses: Vec<JobStatus>,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_by_type_and_statuses(job_type, job_statuses).await
    }

    async fn get_jobs_by_block_number(&self, block_number: u64) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_by_block_number(block_number).await
    }

    async fn get_orphaned_jobs(&self, job_type: &JobType, timeout_seconds: u64) -> Result<Vec<JobItem>, DatabaseError> {
        self.worker_repo.get_orphaned_jobs(job_type, timeout_seconds).await
    }

    async fn get_jobs_by_status(&self, status: JobStatus) -> Result<Vec<JobItem>, DatabaseError> {
        self.job_repo.get_jobs_by_status(status).await
    }

    async fn get_processable_job(&self, job_type: &JobType) -> Result<Option<JobItem>, DatabaseError> {
        self.worker_repo.get_processable_job(job_type).await
    }

    async fn get_verifiable_job(&self, job_type: &JobType) -> Result<Option<JobItem>, DatabaseError> {
        self.worker_repo.get_verifiable_job(job_type).await
    }

    // ================================================================================
    // Worker Methods - delegate to WorkerRepository
    // ================================================================================

    async fn claim_job_for_processing(
        &self,
        job_type: &JobType,
        orchestrator_id: &str,
    ) -> Result<Option<JobItem>, DatabaseError> {
        self.worker_repo.claim_for_processing(job_type, orchestrator_id).await
    }

    async fn claim_job_for_verification(
        &self,
        job_type: &JobType,
        orchestrator_id: &str,
    ) -> Result<Option<JobItem>, DatabaseError> {
        self.worker_repo.claim_for_verification(job_type, orchestrator_id).await
    }

    async fn release_job_claim(&self, job_id: Uuid, delay_seconds: Option<u64>) -> Result<JobItem, DatabaseError> {
        self.worker_repo.release_claim(job_id, delay_seconds).await
    }

    async fn count_jobs_by_type_and_status(&self, job_type: &JobType, status: JobStatus) -> Result<u64, DatabaseError> {
        self.worker_repo.count_by_type_and_status(job_type, status).await
    }

    async fn count_jobs_by_type_and_statuses(
        &self,
        job_type: &JobType,
        statuses: &[JobStatus],
    ) -> Result<u64, DatabaseError> {
        self.worker_repo.count_by_type_and_statuses(job_type, statuses).await
    }

    async fn count_claimed_jobs(&self, orchestrator_id: &str) -> Result<u64, DatabaseError> {
        self.worker_repo.count_claimed(orchestrator_id).await
    }

    async fn count_claimed_jobs_by_type(
        &self,
        orchestrator_id: &str,
        job_type: &JobType,
    ) -> Result<u64, DatabaseError> {
        self.worker_repo.count_claimed_by_type(orchestrator_id, job_type).await
    }

    async fn get_orphaned_verification_jobs(
        &self,
        job_type: &JobType,
        timeout_seconds: u64,
    ) -> Result<Vec<JobItem>, DatabaseError> {
        self.worker_repo.get_orphaned_verification_jobs(job_type, timeout_seconds).await
    }

    // ================================================================================
    // SNOS Batch Management - delegate to BatchRepository
    // ================================================================================

    async fn get_latest_snos_batch(&self) -> Result<Option<SnosBatch>, DatabaseError> {
        self.batch_repo.get_latest_snos_batch().await
    }

    async fn get_snos_batches_by_indices(&self, indexes: Vec<u64>) -> Result<Vec<SnosBatch>, DatabaseError> {
        self.batch_repo.get_snos_batches_by_indices(indexes).await
    }

    async fn update_snos_batch_status_by_index(
        &self,
        index: u64,
        status: SnosBatchStatus,
    ) -> Result<SnosBatch, DatabaseError> {
        self.batch_repo.update_snos_batch_status(index, status).await
    }

    async fn get_snos_batches_by_status(
        &self,
        status: SnosBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        self.batch_repo.get_snos_batches_by_status(status, limit).await
    }

    async fn get_snos_batches_without_jobs(
        &self,
        snos_batch_status: SnosBatchStatus,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        self.batch_repo.get_snos_batches_without_jobs(snos_batch_status).await
    }

    async fn update_or_create_snos_batch(
        &self,
        batch: &SnosBatch,
        update: &SnosBatchUpdates,
    ) -> Result<SnosBatch, DatabaseError> {
        self.batch_repo.upsert_snos_batch(batch, update).await
    }

    async fn create_snos_batch(&self, batch: SnosBatch) -> Result<SnosBatch, DatabaseError> {
        self.batch_repo.create_snos_batch(batch).await
    }

    async fn update_snos_batch(
        &self,
        _filter: Document,
        _update: Document,
        _options: FindOneAndUpdateOptions,
        _start: Instant,
        _index: u64,
    ) -> Result<SnosBatch, DatabaseError> {
        // This is a low-level MongoDB operation not supported in repository pattern
        Err(DatabaseError::UpdateFailed(
            "Raw update_snos_batch not supported - use update_or_create_snos_batch instead".to_string(),
        ))
    }

    // ================================================================================
    // Aggregator Batch Management - delegate to BatchRepository
    // ================================================================================

    async fn get_latest_aggregator_batch(&self) -> Result<Option<AggregatorBatch>, DatabaseError> {
        self.batch_repo.get_latest_aggregator_batch().await
    }

    async fn get_aggregator_batches_by_indexes(
        &self,
        indexes: Vec<u64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError> {
        self.batch_repo.get_aggregator_batches_by_indices(indexes).await
    }

    async fn update_aggregator_batch_status_by_index(
        &self,
        index: u64,
        status: AggregatorBatchStatus,
    ) -> Result<AggregatorBatch, DatabaseError> {
        self.batch_repo.update_aggregator_batch_status(index, status).await
    }

    async fn update_aggregator_batch(
        &self,
        _filter: Document,
        _update: Document,
        _options: FindOneAndUpdateOptions,
        _start: Instant,
        _index: u64,
    ) -> Result<AggregatorBatch, DatabaseError> {
        // This is a low-level MongoDB operation not supported in repository pattern
        Err(DatabaseError::UpdateFailed(
            "Raw update_aggregator_batch not supported - use update_or_create_aggregator_batch instead".to_string(),
        ))
    }

    async fn update_or_create_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        update: &AggregatorBatchUpdates,
    ) -> Result<AggregatorBatch, DatabaseError> {
        self.batch_repo.upsert_aggregator_batch(batch, update).await
    }

    async fn create_aggregator_batch(&self, batch: AggregatorBatch) -> Result<AggregatorBatch, DatabaseError> {
        self.batch_repo.create_aggregator_batch(batch).await
    }

    async fn get_aggregator_batch_for_block(
        &self,
        block_number: u64,
    ) -> Result<Option<AggregatorBatch>, DatabaseError> {
        self.batch_repo.get_aggregator_batch_for_block(block_number).await
    }

    async fn get_start_snos_batch_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Option<SnosBatch>, DatabaseError> {
        self.batch_repo.get_first_snos_batch_for_aggregator(aggregator_index).await
    }

    async fn get_aggregator_batches_by_status(
        &self,
        status: AggregatorBatchStatus,
        limit: Option<i64>,
    ) -> Result<Vec<AggregatorBatch>, DatabaseError> {
        self.batch_repo.get_aggregator_batches_by_status(status, limit).await
    }

    // ================================================================================
    // Batch Relationship Management - delegate to BatchRepository
    // ================================================================================

    async fn get_snos_batches_by_aggregator_index(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        self.batch_repo.get_snos_batches_by_aggregator(aggregator_index).await
    }

    async fn get_open_snos_batches_by_aggregator_index(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        self.batch_repo.get_open_snos_batches_by_aggregator(aggregator_index).await
    }

    async fn count_snos_batches_by_aggregator_batch_index(&self, aggregator_index: u64) -> Result<u64, DatabaseError> {
        self.batch_repo.count_snos_batches_by_aggregator(aggregator_index).await
    }

    async fn get_next_snos_batch_id(&self) -> Result<u64, DatabaseError> {
        self.batch_repo.get_next_snos_batch_id().await
    }

    async fn close_all_snos_batches_for_aggregator(
        &self,
        aggregator_index: u64,
    ) -> Result<Vec<SnosBatch>, DatabaseError> {
        self.batch_repo.close_snos_batches_for_aggregator(aggregator_index).await
    }

    // ================================================================================
    // Health Check
    // ================================================================================

    async fn health_check(&self) -> Result<(), DatabaseError> {
        // All repositories share the same MongoClient, so checking one is sufficient
        // For now, just return Ok - a proper health check could query MongoDB
        Ok(())
    }
}
