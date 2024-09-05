use crate::config::config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::workers::Worker;
use async_trait::async_trait;
use std::error::Error;

pub struct ProvingWorker;

#[async_trait]
impl Worker for ProvingWorker {
    /// 1. Fetch all successful SNOS job runs that don't have a proving job
    /// 2. Create a proving job for each SNOS job run
    async fn run_worker(&self) -> Result<(), Box<dyn Error>> {
        let config = config().await;
        let successful_snos_jobs = config
            .database()
            .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
            .await?;

        for job in successful_snos_jobs {
            create_job(JobType::ProofCreation, job.internal_id.to_string(), job.metadata).await?
        }

        Ok(())
    }
}
