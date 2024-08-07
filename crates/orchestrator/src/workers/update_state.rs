use std::error::Error;

use async_trait::async_trait;

use crate::config::config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::workers::Worker;

pub struct UpdateStateWorker;

#[async_trait]
impl Worker for UpdateStateWorker {
    /// 1. Fetch the last successful state update job
    /// 2. Fetch all successful proving jobs covering blocks after the last state update
    /// 3. Create state updates for all the blocks that don't have a state update job
    async fn run_worker(&self) -> Result<(), Box<dyn Error>> {
        let config = config().await;
        let latest_successful_job =
            config.database().get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await?;

        match latest_successful_job {
            Some(job) => {
                let latest_successful_job_internal_id = job.internal_id;

                let successful_proving_jobs = config
                    .database()
                    .get_jobs_after_internal_id_by_job_type(
                        JobType::ProofCreation,
                        JobStatus::Completed,
                        latest_successful_job_internal_id,
                    )
                    .await?;

                for job in successful_proving_jobs {
                    create_job(JobType::StateTransition, job.internal_id, job.metadata).await?;
                }

                Ok(())
            }
            None => {
                log::info!("No successful state update jobs found");
                return Ok(());
            }
        }
    }
}
