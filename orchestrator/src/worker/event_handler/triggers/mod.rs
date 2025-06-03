pub(crate) mod batching;
pub(crate) mod data_submission_worker;
pub(crate) mod proof_registration;
pub(crate) mod proving;
pub(crate) mod snos;
pub(crate) mod update_state;
mod aggregator;

use crate::core::config::Config;
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
    // block we will resolve the existing failed job first.

    // We assume the system to keep working till a job hasn't failed,
    // as soon as it fails we currently halt any more execution and wait for manual intervention.

    // Checks if any of the jobs have failed
    // Failure : JobStatus::VerificationFailed, JobStatus::VerificationTimeout, JobStatus::Failed
    // Halts any new job creation till all the count of failed jobs is not Zero.
    async fn is_worker_enabled(&self, config: Arc<Config>) -> color_eyre::Result<bool> {
        let failed_jobs = config
            .database()
            .get_jobs_by_statuses(vec![JobStatus::Failed, JobStatus::VerificationTimeout], Some(1))
            .await?;

        if !failed_jobs.is_empty() {
            return Ok(false);
        }

        Ok(true)
    }
}
