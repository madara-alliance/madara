pub(crate) mod aggregator;
pub(crate) mod aggregator_batching;
pub(crate) mod batching;
pub(crate) mod data_submission_worker;
pub(crate) mod proof_registration;
pub(crate) mod proving;
pub(crate) mod snos;
pub(crate) mod snos_batching;
pub(crate) mod update_state;

use crate::core::config::Config;
use crate::types::constant::ORCHESTRATOR_VERSION;
use crate::types::jobs::types::JobStatus;
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
    // block, we will resolve the existing failed job first.

    // We assume the system to keep working till a job hasn't failed.
    // As soon as it fails, we currently halt any more execution and wait for manual intervention.

    // Checks if any of the jobs have failed (empty job_type vector implies any job)
    // Failure: JobStatus::VerificationFailed, JobStatus::VerificationTimeout, JobStatus::Failed
    // Halts any new job creation till all the count of failed jobs is not Zero.
    async fn is_worker_enabled(&self, config: Arc<Config>) -> color_eyre::Result<bool> {
        let failed_jobs = config
            .database()
            .get_jobs_by_types_and_statuses(
                vec![],
                vec![JobStatus::Failed, JobStatus::VerificationTimeout],
                None,
                Some(ORCHESTRATOR_VERSION.to_string()),
            )
            .await?;

        if !failed_jobs.is_empty() {
            // Group jobs by status to provide accurate logging
            let failed_count = failed_jobs.iter().filter(|j| j.status == JobStatus::Failed).count();
            let timeout_count = failed_jobs.iter().filter(|j| j.status == JobStatus::VerificationTimeout).count();

            let status_summary = match (failed_count > 0, timeout_count > 0) {
                (true, true) => format!("{} Failed, {} VerificationTimeout", failed_count, timeout_count),
                (true, false) => format!("{} Failed", failed_count),
                (false, true) => format!("{} VerificationTimeout", timeout_count),
                (false, false) => "0".to_string(),
            };

            tracing::warn!(
                "There are {} jobs in the DB ({}). Not creating new jobs to prevent inconsistencies (existing jobs will be processed). Please manually fix the failed jobs before continuing!",
                failed_jobs.len(),
                status_summary
            );
            return Ok(false);
        }

        Ok(true)
    }
}
