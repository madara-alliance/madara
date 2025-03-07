use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::config::Config;
use crate::jobs::types::JobStatus;

pub mod data_submission_worker;
pub mod proof_registration;
pub mod proving;
pub mod snos;
pub mod update_state;

#[derive(Error, Debug)]
pub enum WorkerError {
    #[error("Worker execution failed: {0}")]
    ExecutionError(String),

    #[error("JSON RPC error: {0}")]
    JsonRpcError(String),

    #[error("Other error: {0}")]
    Other(Box<dyn Error + Send + Sync>),
}

#[async_trait]
pub trait Worker: Send + Sync {
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
