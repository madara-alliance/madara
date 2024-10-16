use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::workers::Worker;

pub struct ProvingWorker;

#[async_trait]
impl Worker for ProvingWorker {
    /// 1. Fetch all successful SNOS job runs that don't have a proving job
    /// 2. Create a proving job for each SNOS job run
    async fn run_worker(&self, config: Arc<Config>) -> Result<(), Box<dyn Error>> {
        tracing::trace!(log_type = "starting", category = "ProvingWorker", "ProvingWorker started.");

        let successful_snos_jobs = config
            .database()
            .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
            .await?;

        tracing::debug!("Found {} successful SNOS jobs without proving jobs", successful_snos_jobs.len());

        for job in successful_snos_jobs {
            tracing::debug!(job_id = %job.internal_id, "Creating proof creation job for SNOS job");
            match create_job(JobType::ProofCreation, job.internal_id.to_string(), job.metadata.clone(), config.clone())
                .await
            {
                Ok(_) => tracing::info!(block_no = %job.internal_id, "Successfully created proof creation job"),
                Err(e) => tracing::error!(job_id = %job.internal_id, error = %e, "Failed to create proof creation job"),
            }
        }

        tracing::trace!(log_type = "completed", category = "ProvingWorker", "ProvingWorker completed.");
        Ok(())
    }
}
