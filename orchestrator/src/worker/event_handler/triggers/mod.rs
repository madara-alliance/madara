pub(crate) mod aggregator;
pub(crate) mod batching;
pub(crate) mod data_submission_worker;
pub(crate) mod proof_registration;
pub(crate) mod proving;
pub(crate) mod snos;
pub(crate) mod update_state;

use crate::core::config::Config;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait JobTrigger: Send + Sync {
    async fn run_worker_if_enabled(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        if !self.is_worker_enabled(config.clone()).await? {
            return Ok(());
        }
        self.run_worker(config).await
    }

    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()>;

    // Assumption
    // If say a job for block X fails, we don't want the worker to respawn another job for the same
    // block we will resolve the existing failed job first.

    // We assume the system to keep working till a job hasn't failed,
    // as soon as it fails we currently halt any more execution and wait for manual intervention.

    // Checks if any of the jobs have failed
    // Failure : JobStatus::VerificationFailed, JobStatus::VerificationTimeout, JobStatus::Failed
    // Halts any new job creation till all the count of failed jobs is not Zero.
    async fn is_worker_enabled(&self, config: Arc<Config>) -> color_eyre::Result<bool> {
        let failed_jobs = config
            .database()
            .get_jobs_by_types_and_statuses(vec![], vec![JobStatus::Failed, JobStatus::VerificationTimeout], Some(1))
            .await?;

        if !failed_jobs.is_empty() {
            return Ok(false);
        }

        Ok(true)
    }

    /// Self-healing mechanism to recover orphaned jobs for a specific job type.
    ///
    /// This method finds jobs that are stuck in `LockedForProcessing` status beyond the configured
    /// timeout and resets them to `Created` status so they can be processed again.
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database access and timeout settings
    /// * `job_type` - The type of job to heal (SNOS, Proving, etc.)
    ///
    /// # Returns
    /// * `Result<u32>` - Number of jobs healed or an error
    ///
    /// # Behavior
    /// - Only heals jobs that have been locked for longer than the configured timeout
    /// - Resets job status from `LockedForProcessing` to `Created`
    /// - Clears the `process_started_at` timestamp to allow fresh processing
    /// - Logs recovery actions for monitoring and debugging
    async fn heal_orphaned_jobs(&self, config: Arc<Config>, job_type: JobType) -> anyhow::Result<u32> {
        let timeout_seconds = config.service_config().job_processing_timeout_seconds;
        let orphaned_jobs = config.database().get_orphaned_jobs(&job_type, timeout_seconds).await?;

        if orphaned_jobs.is_empty() {
            return Ok(0);
        }

        tracing::warn!(
            job_type = ?job_type,
            orphaned_count = orphaned_jobs.len(),
            timeout_seconds = timeout_seconds,
            "Found orphaned jobs, initiating self-healing recovery"
        );

        let mut healed_count = 0;

        for mut job in orphaned_jobs {
            job.metadata.common.process_started_at = None;

            let update_result = config
                .database()
                .update_job(
                    &job,
                    JobItemUpdates::new()
                        .update_status(JobStatus::Created)
                        .update_metadata(job.metadata.clone())
                        .build(),
                )
                .await;

            match update_result {
                Ok(_) => {
                    healed_count += 1;
                    tracing::info!(
                        job_id = %job.id,
                        job_type = ?job_type,
                        internal_id = %job.internal_id,
                        "Successfully healed orphaned job - reset from LockedForProcessing to Created"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        job_id = %job.id,
                        job_type = ?job_type,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Failed to heal orphaned job"
                    );
                }
            }
        }

        if healed_count > 0 {
            tracing::warn!(
                job_type = ?job_type,
                healed_count = healed_count,
                "Self-healing completed: recovered orphaned jobs"
            );
        }

        Ok(healed_count)
    }
}
