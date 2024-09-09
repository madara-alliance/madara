use std::error::Error;

use async_trait::async_trait;

use crate::config::config;
use crate::jobs::constants::JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY;
use crate::jobs::create_job;
use crate::jobs::types::{JobItem, JobStatus, JobType};
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

                let mut metadata = job.metadata;
                metadata.insert(
                    JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY.to_string(),
                    Self::parse_job_items_into_block_number_list(successful_proving_jobs.clone()),
                );

                // Creating a single job for all the pending blocks.
                create_job(JobType::StateTransition, successful_proving_jobs[0].internal_id.clone(), metadata).await?;

                Ok(())
            }
            None => {
                log::info!("No successful state update jobs found");
                return Ok(());
            }
        }
    }
}

impl UpdateStateWorker {
    /// To parse the block numbers from the vector of jobs.
    pub fn parse_job_items_into_block_number_list(job_items: Vec<JobItem>) -> String {
        job_items.iter().map(|j| j.internal_id.clone()).collect::<Vec<String>>().join(",")
    }
}

#[cfg(test)]
mod test_update_state_worker_utils {
    use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
    use crate::workers::update_state::UpdateStateWorker;
    use chrono::{SubsecRound, Utc};
    use rstest::rstest;
    use uuid::Uuid;

    #[rstest]
    fn test_parse_job_items_into_block_number_list() {
        let mut job_vec = Vec::new();
        for i in 0..3 {
            job_vec.push(JobItem {
                id: Uuid::new_v4(),
                internal_id: i.to_string(),
                job_type: JobType::ProofCreation,
                status: JobStatus::Completed,
                external_id: ExternalId::Number(0),
                metadata: Default::default(),
                version: 0,
                created_at: Utc::now().round_subsecs(0),
                updated_at: Utc::now().round_subsecs(0),
            });
        }

        let block_string = UpdateStateWorker::parse_job_items_into_block_number_list(job_vec);
        assert_eq!(block_string, String::from("0,1,2"));
    }
}
