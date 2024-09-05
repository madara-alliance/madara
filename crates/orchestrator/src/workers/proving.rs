use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::workers::Worker;
use async_trait::async_trait;
use std::error::Error;
use std::sync::Arc;

pub struct ProvingWorker;

#[async_trait]
impl Worker for ProvingWorker {
    /// 1. Fetch all successful SNOS job runs that don't have a proving job
    /// 2. Create a proving job for each SNOS job run
    async fn run_worker(&self, config: Arc<Config>) -> Result<(), Box<dyn Error>> {
        let successful_snos_jobs = config
            .database()
            .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
            .await?;

        for job in successful_snos_jobs {
            create_job(JobType::ProofCreation, job.internal_id.to_string(), job.metadata, config.clone()).await?
        }

        Ok(())
    }
}
