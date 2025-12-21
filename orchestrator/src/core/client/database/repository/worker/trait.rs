use async_trait::async_trait;
use uuid::Uuid;

use crate::core::client::database::error::DatabaseError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};

/// Repository for worker-related operations (claiming, releasing, orphan detection)
///
/// This repository handles the queue-less architecture where workers poll MongoDB
/// directly instead of consuming from SQS queues.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WorkerRepository: Send + Sync {
    // =========================================================================
    // Job Discovery (Read-only)
    // =========================================================================

    /// Find a job ready for processing (Created or PendingRetryProcessing)
    /// Does NOT claim the job - just finds it
    async fn get_processable_job(&self, job_type: &JobType) -> Result<Option<JobItem>, DatabaseError>;

    /// Find a job ready for verification (Processed or PendingRetryVerification)
    /// Does NOT claim the job - just finds it
    async fn get_verifiable_job(&self, job_type: &JobType) -> Result<Option<JobItem>, DatabaseError>;

    // =========================================================================
    // Atomic Claiming Operations
    // =========================================================================

    /// Atomically claim a job for processing
    ///
    /// Uses findOneAndUpdate to atomically find and claim. A job is claimable if:
    /// - Status is Created or PendingRetryProcessing
    /// - available_at is None or in the past
    /// - claimed_by is None
    ///
    /// Priority: PendingRetryProcessing > Created, then FIFO by created_at
    async fn claim_for_processing(
        &self,
        job_type: &JobType,
        orchestrator_id: &str,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// Atomically claim a job for verification
    ///
    /// Uses findOneAndUpdate to atomically find and claim. A job is claimable if:
    /// - Status is Processed
    /// - available_at is None or in the past
    /// - claimed_by is None
    ///
    /// Priority: FIFO by created_at
    async fn claim_for_verification(
        &self,
        job_type: &JobType,
        orchestrator_id: &str,
    ) -> Result<Option<JobItem>, DatabaseError>;

    /// Release a job claim, optionally with a delay before it becomes available again
    ///
    /// Clears claimed_by and sets available_at if delay is specified.
    async fn release_claim(&self, job_id: Uuid, delay_seconds: Option<u64>) -> Result<JobItem, DatabaseError>;

    // =========================================================================
    // Orphan Detection (Self-Healing)
    // =========================================================================

    /// Get jobs stuck in LockedForProcessing or LockedForVerification
    ///
    /// These are jobs claimed by orchestrators that crashed before completing.
    async fn get_orphaned_jobs(&self, job_type: &JobType, timeout_seconds: u64) -> Result<Vec<JobItem>, DatabaseError>;

    /// Get jobs stuck in Processed with claimed_by set (verification orphans)
    ///
    /// These are jobs claimed for verification but the orchestrator crashed.
    async fn get_orphaned_verification_jobs(
        &self,
        job_type: &JobType,
        timeout_seconds: u64,
    ) -> Result<Vec<JobItem>, DatabaseError>;

    // =========================================================================
    // Counting Operations (Concurrency Control)
    // =========================================================================

    /// Count jobs by type and status
    async fn count_by_type_and_status(&self, job_type: &JobType, status: JobStatus) -> Result<u64, DatabaseError>;

    /// Count jobs by type with multiple statuses
    async fn count_by_type_and_statuses(
        &self,
        job_type: &JobType,
        statuses: &[JobStatus],
    ) -> Result<u64, DatabaseError>;

    /// Count all jobs claimed by an orchestrator
    async fn count_claimed(&self, orchestrator_id: &str) -> Result<u64, DatabaseError>;

    /// Count jobs of a specific type claimed by an orchestrator
    async fn count_claimed_by_type(&self, orchestrator_id: &str, job_type: &JobType) -> Result<u64, DatabaseError>;
}
