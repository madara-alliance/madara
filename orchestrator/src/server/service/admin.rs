use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::service::JobService;

/// Result of a bulk job operation
#[derive(Debug, Clone)]
pub struct BulkJobResult {
    /// Number of jobs successfully queued
    pub success_count: u64,
    /// IDs of jobs that were successfully queued
    pub successful_job_ids: Vec<Uuid>,
    /// Number of jobs that failed to queue
    pub failed_count: u64,
    /// IDs of jobs that failed to queue
    pub failed_job_ids: Vec<Uuid>,
}

/// Admin service for bulk job operations
pub struct AdminService;

impl AdminService {
    /// Retry all failed jobs, optionally filtered by job types
    ///
    /// # Arguments
    /// * `job_types` - Optional vector of job types to filter by (empty vec = all types)
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<BulkJobResult, JobError>` - Result containing success and failure counts/IDs
    pub async fn retry_all_failed_jobs(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        info!(
            job_types = ?job_types,
            "Admin: Starting retry of all failed jobs"
        );

        // Query all failed jobs with optional type filter
        let failed_jobs = config
            .database()
            .get_jobs_by_types_and_statuses(job_types.clone(), vec![JobStatus::Failed], None)
            .await?;

        let total_jobs = failed_jobs.len();
        info!(
            job_types = ?job_types,
            count = total_jobs,
            "Admin: Found failed jobs to retry"
        );

        let mut successful_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();

        // Retry each failed job
        for job in failed_jobs {
            match JobHandlerService::retry_job(job.id, config.clone()).await {
                Ok(_) => {
                    successful_job_ids.push(job.id);
                    info!(
                        job_id = %job.id,
                        job_type = ?job.job_type,
                        internal_id = %job.internal_id,
                        "Admin: Successfully retried failed job"
                    );
                }
                Err(e) => {
                    failed_job_ids.push(job.id);
                    error!(
                        job_id = %job.id,
                        job_type = ?job.job_type,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Admin: Failed to retry job - job remains in Failed status"
                    );
                    // Continue with other jobs even if one fails
                }
            }
        }

        let result = BulkJobResult {
            success_count: successful_job_ids.len() as u64,
            successful_job_ids,
            failed_count: failed_job_ids.len() as u64,
            failed_job_ids: failed_job_ids.clone(),
        };

        if result.failed_count > 0 {
            warn!(
                job_types = ?job_types,
                total = total_jobs,
                success = result.success_count,
                failed = result.failed_count,
                failed_job_ids = ?failed_job_ids,
                "Admin: Completed retry of failed jobs with some failures"
            );
        } else {
            info!(
                job_types = ?job_types,
                total = total_jobs,
                success = result.success_count,
                "Admin: Completed retry of failed jobs - all successful"
            );
        }

        Ok(result)
    }

    /// Re-add all PendingVerification jobs to verification queue
    ///
    /// # Arguments
    /// * `job_types` - Optional vector of job types to filter by (empty vec = all types)
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<BulkJobResult, JobError>` - Result containing success and failure counts/IDs
    pub async fn requeue_pending_verification(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        info!(
            job_types = ?job_types,
            "Admin: Starting requeue of pending verification jobs"
        );

        // Query all PendingVerification jobs with optional type filter
        let jobs = config
            .database()
            .get_jobs_by_types_and_statuses(job_types.clone(), vec![JobStatus::PendingVerification], None)
            .await?;

        let total_jobs = jobs.len();
        info!(
            job_types = ?job_types,
            count = total_jobs,
            "Admin: Found pending verification jobs to requeue"
        );

        let mut successful_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();

        // Queue each job to appropriate verification queue
        for job in jobs {
            match JobService::add_job_to_verify_queue(config.clone(), job.id, &job.job_type, None).await {
                Ok(_) => {
                    successful_job_ids.push(job.id);
                    info!(
                        job_id = %job.id,
                        job_type = ?job.job_type,
                        internal_id = %job.internal_id,
                        "Admin: Successfully re-queued job for verification"
                    );
                }
                Err(e) => {
                    failed_job_ids.push(job.id);
                    error!(
                        job_id = %job.id,
                        job_type = ?job.job_type,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Admin: Failed to queue job for verification - job remains in PendingVerification status"
                    );
                    // Continue with other jobs even if one fails
                }
            }
        }

        let result = BulkJobResult {
            success_count: successful_job_ids.len() as u64,
            successful_job_ids,
            failed_count: failed_job_ids.len() as u64,
            failed_job_ids: failed_job_ids.clone(),
        };

        if result.failed_count > 0 {
            warn!(
                job_types = ?job_types,
                total = total_jobs,
                success = result.success_count,
                failed = result.failed_count,
                failed_job_ids = ?failed_job_ids,
                "Admin: Completed requeue of pending verification jobs with some failures"
            );
        } else {
            info!(
                job_types = ?job_types,
                total = total_jobs,
                success = result.success_count,
                "Admin: Completed requeue of pending verification jobs - all successful"
            );
        }

        Ok(result)
    }

    /// Re-add all Created jobs to processing queue
    ///
    /// # Arguments
    /// * `job_types` - Optional vector of job types to filter by (empty vec = all types)
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<BulkJobResult, JobError>` - Result containing success and failure counts/IDs
    pub async fn requeue_created_jobs(
        job_types: Vec<JobType>,
        config: Arc<Config>,
    ) -> Result<BulkJobResult, JobError> {
        info!(
            job_types = ?job_types,
            "Admin: Starting requeue of created jobs"
        );

        // Query all Created jobs with optional type filter
        let jobs = config
            .database()
            .get_jobs_by_types_and_statuses(job_types.clone(), vec![JobStatus::Created], None)
            .await?;

        let total_jobs = jobs.len();
        info!(
            job_types = ?job_types,
            count = total_jobs,
            "Admin: Found created jobs to requeue"
        );

        let mut successful_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();

        // Queue each job to appropriate processing queue
        for job in jobs {
            match JobService::add_job_to_process_queue(job.id, &job.job_type, config.clone()).await {
                Ok(_) => {
                    successful_job_ids.push(job.id);
                    info!(
                        job_id = %job.id,
                        job_type = ?job.job_type,
                        internal_id = %job.internal_id,
                        "Admin: Successfully re-queued created job for processing"
                    );
                }
                Err(e) => {
                    failed_job_ids.push(job.id);
                    error!(
                        job_id = %job.id,
                        job_type = ?job.job_type,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Admin: Failed to queue job for processing - job remains in Created status"
                    );
                    // Continue with other jobs even if one fails
                }
            }
        }

        let result = BulkJobResult {
            success_count: successful_job_ids.len() as u64,
            successful_job_ids,
            failed_count: failed_job_ids.len() as u64,
            failed_job_ids: failed_job_ids.clone(),
        };

        if result.failed_count > 0 {
            warn!(
                job_types = ?job_types,
                total = total_jobs,
                success = result.success_count,
                failed = result.failed_count,
                failed_job_ids = ?failed_job_ids,
                "Admin: Completed requeue of created jobs with some failures"
            );
        } else {
            info!(
                job_types = ?job_types,
                total = total_jobs,
                success = result.success_count,
                "Admin: Completed requeue of created jobs - all successful"
            );
        }

        Ok(result)
    }
}
