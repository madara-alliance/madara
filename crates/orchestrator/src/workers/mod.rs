use crate::{config::config, jobs::types::JobStatus};
use async_trait::async_trait;
use std::error::Error;

pub mod data_submission_worker;
pub mod proof_registration;
pub mod proving;
pub mod snos;
pub mod update_state;

#[async_trait]
pub trait Worker: Send + Sync {
    async fn run_worker_if_enabled(&self) -> Result<(), Box<dyn Error>> {
        if !self.is_worker_enabled().await? {
            return Ok(());
        }
        self.run_worker().await
    }

    async fn run_worker(&self) -> Result<(), Box<dyn Error>>;

    // Assumption
    // If say a job for block X fails, we don't want the worker to respawn another job for the same block
    // we will resolve the existing failed job first.

    // We assume the system to keep working till a job hasn't failed,
    // as soon as it fails we currently halt any more execution and wait for manual intervention.

    // Checks if any of the jobs have failed
    // Failure : JobStatus::VerificationFailed, JobStatus::VerificationTimeout, JobStatus::Failed
    // Halts any new job creation till all the count of failed jobs is not Zero.
    async fn is_worker_enabled(&self) -> Result<bool, Box<dyn Error>> {
        let config = config().await;

        let failed_jobs = config
            .database()
            .get_jobs_by_statuses(vec![JobStatus::VerificationFailed, JobStatus::VerificationTimeout], Some(1))
            .await?;

        if !failed_jobs.is_empty() {
            return Ok(false);
        }

        Ok(true)
    }
}
