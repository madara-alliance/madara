use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::eyre;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;

use crate::core::config::Config;
use crate::types::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;

pub struct UpdateStateJobTrigger;

#[async_trait]
impl JobTrigger for UpdateStateJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "UpdateStateWorker", "UpdateStateWorker started.");

        // Self-healing: recover any orphaned StateTransition jobs before creating new ones
        if let Err(e) = self.heal_orphaned_jobs(config.clone(), JobType::StateTransition).await {
            tracing::error!(error = %e, "Failed to heal orphaned StateTransition jobs, continuing with normal processing");
        }

        let latest_job = config.database().get_latest_job_by_type(JobType::StateTransition).await?;
        let (completed_da_jobs, last_block_processed_in_last_job) = match latest_job {
            Some(job) => {
                if job.status != JobStatus::Completed {
                    tracing::warn!(
                        "There's already a pending update state job. Parallel jobs can cause nonce issues or can \
                         completely fail as the update logic needs to be strictly ordered. Returning safely..."
                    );
                    return Ok(());
                }

                // Extract blocks from state transition metadata
                let state_metadata: StateUpdateMetadata = job.metadata.specific
                    .try_into()
                    .map_err(|e| {
                        tracing::error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for state transition job");
                        e
                    })?;

                let mut blocks_processed = state_metadata.blocks_to_settle.clone();
                blocks_processed.sort();

                let last_block_processed = *blocks_processed
                    .last()
                    .ok_or_else(|| eyre!("No blocks found in previous state transition job"))?;

                (
                    config
                        .database()
                        .get_jobs_after_internal_id_by_job_type(
                            JobType::DataSubmission,
                            JobStatus::Completed,
                            last_block_processed.to_string(),
                        )
                        .await?,
                    Some(last_block_processed),
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
            tracing::warn!("No DA jobs completed after the last settled block. Returning safely...");
            return Ok(());
        }

        // Verify block continuity
        match last_block_processed_in_last_job {
            Some(last_block) => {
                if blocks_to_process[0] != last_block + 1 {
                    tracing::warn!(
                        "DA job for the block just after the last settled block is not yet completed. Returning \
                         safely..."
                    );
                    return Ok(());
                }
            }
            None => {
                let min_block_to_process = config.service_config().min_block_to_process;
                if blocks_to_process[0] != min_block_to_process {
                    tracing::warn!("DA job for the first block is not yet completed. Returning safely...");
                    return Ok(());
                }
            }
        }

        let blocks_to_process = find_successive_blocks_in_vector(blocks_to_process, Some(10));

        // Prepare state transition metadata
        let mut state_metadata = StateUpdateMetadata {
            blocks_to_settle: blocks_to_process.clone(),
            snos_output_paths: Vec::new(),
            program_output_paths: Vec::new(),
            blob_data_paths: Vec::new(),
            last_failed_block_no: None,
            tx_hashes: Vec::new(),
        };

        // Collect paths from SNOS and DA jobs
        for block_number in &blocks_to_process {
            // Get SNOS job paths
            let snos_job = config
                .database()
                .get_job_by_internal_id_and_type(&block_number.to_string(), &JobType::SnosRun)
                .await?
                .ok_or_else(|| eyre!("SNOS job not found for block {}", block_number))?;
            let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().map_err(|e| {
                tracing::error!(job_id = %snos_job.internal_id, error = %e, "Invalid metadata type for SNOS job");
                e
            })?;

            if let Some(snos_path) = &snos_metadata.snos_output_path {
                state_metadata.snos_output_paths.push(snos_path.clone());
            }
            if let Some(program_path) = &snos_metadata.program_output_path {
                state_metadata.program_output_paths.push(program_path.clone());
            }

            // Get DA job blob path
            let da_job = config
                .database()
                .get_job_by_internal_id_and_type(&block_number.to_string(), &JobType::DataSubmission)
                .await?
                .ok_or_else(|| eyre!("DA job not found for block {}", block_number))?;

            let da_metadata: DaMetadata = da_job.metadata.specific.try_into().map_err(|e| {
                tracing::error!(job_id = %da_job.internal_id, error = %e, "Invalid metadata type for DA job");
                e
            })?;

            if let Some(blob_path) = &da_metadata.blob_data_path {
                state_metadata.blob_data_paths.push(blob_path.clone());
            }
        }
        // Create job metadata
        let metadata = JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(state_metadata),
        };

        // Create the state transition job
        let new_job_id = blocks_to_process[0].to_string();
        match JobHandlerService::create_job(JobType::StateTransition, new_job_id.clone(), metadata, config.clone())
            .await
        {
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
pub fn find_successive_blocks_in_vector(block_numbers: Vec<u64>, limit: Option<usize>) -> Vec<u64> {
    block_numbers
        .iter()
        .enumerate()
        .take_while(|(index, block_number)| {
            **block_number == (block_numbers[0] + *index as u64) && (limit.is_none() || *index <= limit.unwrap())
        })
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
        let result = super::find_successive_blocks_in_vector(input, None);
        assert_eq!(result, expected);
    }
}
