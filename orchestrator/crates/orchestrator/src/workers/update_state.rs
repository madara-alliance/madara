use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::KeyValue;

use crate::config::Config;
use crate::jobs::constants::JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::workers::Worker;

pub struct UpdateStateWorker;

#[async_trait]
impl Worker for UpdateStateWorker {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "UpdateStateWorker", "UpdateStateWorker started.");

        let latest_job = config.database().get_latest_job_by_type(JobType::StateTransition).await?;

        let (completed_da_jobs, last_block_processed_in_last_job) = match latest_job {
            Some(job) => {
                if job.status != JobStatus::Completed {
                    log::warn!(
                        "There's already a pending update state job. Parallel jobs can cause nonce issues or can \
                         completely fail as the update logic needs to be strictly ordered. Returning safely..."
                    );
                    return Ok(());
                }

                let mut blocks_processed_in_last_job: Vec<u64> = job
                    .metadata
                    .get(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY)
                    .unwrap()
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect();

                // ideally it's already sorted, but just to be safe
                blocks_processed_in_last_job.sort();

                let last_block_processed_in_last_job =
                    blocks_processed_in_last_job[blocks_processed_in_last_job.len() - 1];

                (
                    config
                        .database()
                        .get_jobs_after_internal_id_by_job_type(
                            JobType::DataSubmission,
                            JobStatus::Completed,
                            last_block_processed_in_last_job.to_string(),
                        )
                        .await?,
                    Some(last_block_processed_in_last_job),
                )
            }
            None => {
                tracing::warn!("No previous state transition job found, fetching latest data submission job");
                // Getting latest DA job in case no latest state update job is present
                (
                    config
                        .database()
                        .get_jobs_without_successor(
                            JobType::DataSubmission,
                            JobStatus::Completed,
                            JobType::StateTransition,
                        )
                        .await?,
                    None,
                )
            }
        };

        let mut blocks_to_process: Vec<u64> =
            completed_da_jobs.iter().map(|j| j.internal_id.parse::<u64>().unwrap()).collect();
        blocks_to_process.sort();

        // no DA jobs completed after the last settled block
        if blocks_to_process.is_empty() {
            log::warn!("No DA jobs completed after the last settled block. Returning safely...");
            return Ok(());
        }

        match last_block_processed_in_last_job {
            Some(last_block_processed_in_last_job) => {
                // DA job for the block just after the last settled block
                // is not yet completed
                if blocks_to_process[0] != last_block_processed_in_last_job + 1 {
                    log::warn!(
                        "DA job for the block just after the last settled block is not yet completed. Returning \
                         safely..."
                    );
                    return Ok(());
                }
            }
            None => {
                if blocks_to_process[0] != 0 {
                    log::warn!("DA job for the first block is not yet completed. Returning safely...");
                    return Ok(());
                }
            }
        }

        let mut blocks_to_process: Vec<u64> = find_successive_blocks_in_vector(blocks_to_process);

        if blocks_to_process.len() > 10 {
            blocks_to_process = blocks_to_process.into_iter().take(10).collect();
        }

        let mut metadata = HashMap::new();
        metadata.insert(
            JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY.to_string(),
            blocks_to_process.iter().map(|ele| ele.to_string()).collect::<Vec<String>>().join(","),
        );

        // Creating a single job for all the pending blocks.
        let new_job_id = blocks_to_process[0].to_string();
        match create_job(JobType::StateTransition, new_job_id.clone(), metadata, config.clone()).await {
            Ok(_) => tracing::info!(block_id = %new_job_id, "Successfully created new state transition job"),
            Err(e) => {
                tracing::error!(job_id = %new_job_id, error = %e, "Failed to create new state transition job");
                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::StateTransition)),
                    KeyValue::new("operation_type", format!("{:?}", "create_job")),
                ];
                ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                return Err(e.into());
            }
        }

        tracing::trace!(log_type = "completed", category = "UpdateStateWorker", "UpdateStateWorker completed.");
        Ok(())
    }
}

/// Gets the successive list of blocks from all the blocks processed in previous jobs
/// Eg : input_vec : [1,2,3,4,7,8,9,11]
/// We will take the first 4 block numbers and send it for processing
pub fn find_successive_blocks_in_vector(block_numbers: Vec<u64>) -> Vec<u64> {
    block_numbers
        .iter()
        .enumerate()
        .take_while(|(index, block_number)| **block_number == (block_numbers[0] + *index as u64))
        .map(|(_, block_number)| *block_number)
        .collect()
}

#[cfg(test)]
mod test_update_state_worker_utils {
    use rstest::rstest;

    #[rstest]
    #[case(vec![], vec![])]
    #[case(vec![1], vec![1])]
    #[case(vec![1, 2, 3, 4, 5], vec![1, 2, 3, 4, 5])]
    #[case(vec![1, 2, 3, 4, 7, 8, 9, 11], vec![1, 2, 3, 4])]
    #[case(vec![1, 3, 5, 7, 9], vec![1])]
    fn test_find_successive_blocks(#[case] input: Vec<u64>, #[case] expected: Vec<u64>) {
        let result = super::find_successive_blocks_in_vector(input);
        assert_eq!(result, expected);
    }
}
