use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
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
    async fn run_worker(&self, config: Arc<Config>) -> Result<(), Box<dyn Error>> {
        tracing::info!(log_type = "starting", category = "UpdateStateWorker", "UpdateStateWorker started.");

        let latest_successful_job =
            config.database().get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await?;

        match latest_successful_job {
            Some(job) => {
                tracing::debug!(job_id = %job.id, "Found latest successful state transition job");
                let successful_da_jobs_without_successor = config
                    .database()
                    .get_jobs_without_successor(JobType::DataSubmission, JobStatus::Completed, JobType::StateTransition)
                    .await?;

                if successful_da_jobs_without_successor.is_empty() {
                    tracing::debug!("No new data submission jobs to process");
                    return Ok(());
                }

                tracing::debug!(
                    count = successful_da_jobs_without_successor.len(),
                    "Found data submission jobs without state transition"
                );

                let mut metadata = job.metadata;
                let blocks_to_settle =
                    Self::parse_job_items_into_block_number_list(successful_da_jobs_without_successor.clone());
                metadata.insert(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY.to_string(), blocks_to_settle.clone());

                tracing::trace!(blocks_to_settle = %blocks_to_settle, "Prepared blocks to settle for state transition");

                // Creating a single job for all the pending blocks.
                let new_job_id = successful_da_jobs_without_successor[0].internal_id.clone();
                match create_job(JobType::StateTransition, new_job_id.clone(), metadata, config).await {
                    Ok(_) => tracing::info!(job_id = %new_job_id, "Successfully created new state transition job"),
                    Err(e) => {
                        tracing::error!(job_id = %new_job_id, error = %e, "Failed to create new state transition job");
                        return Err(e.into());
                    }
                }

                tracing::info!(log_type = "completed", category = "UpdateStateWorker", "UpdateStateWorker completed.");
                Ok(())
            }
            None => {
                tracing::warn!("No previous state transition job found, fetching latest data submission job");
                // Getting latest DA job in case no latest state update job is present
                let latest_successful_jobs_without_successor = config
                    .database()
                    .get_jobs_without_successor(JobType::DataSubmission, JobStatus::Completed, JobType::StateTransition)
                    .await?;

                if latest_successful_jobs_without_successor.is_empty() {
                    tracing::debug!("No data submission jobs found to process");
                    return Ok(());
                }

                let job = latest_successful_jobs_without_successor[0].clone();
                let mut metadata = job.metadata;

                let blocks_to_settle =
                    Self::parse_job_items_into_block_number_list(latest_successful_jobs_without_successor.clone());
                metadata.insert(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY.to_string(), blocks_to_settle.clone());

                tracing::trace!(job_id = %job.id, blocks_to_settle = %blocks_to_settle, "Prepared blocks to settle for initial state transition");

                match create_job(JobType::StateTransition, job.internal_id.clone(), metadata, config).await {
                    Ok(_) => tracing::info!(job_id = %job.id, "Successfully created initial state transition job"),
                    Err(e) => {
                        tracing::error!(job_id = %job.id, error = %e, "Failed to create initial state transition job");
                        return Err(e.into());
                    }
                }

                tracing::info!(log_type = "completed", category = "UpdateStateWorker", "UpdateStateWorker completed.");
                Ok(())
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
    use chrono::{SubsecRound, Utc};
    use rstest::rstest;
    use uuid::Uuid;

    use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
    use crate::workers::update_state::UpdateStateWorker;

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
