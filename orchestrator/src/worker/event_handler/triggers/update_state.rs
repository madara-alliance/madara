use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::eyre;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;

use crate::core::config::Config;
use crate::types::jobs::metadata::{
    AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;

pub struct UpdateStateJobTrigger;

#[async_trait]
impl JobTrigger for UpdateStateJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        match config.layer() {
            Layer::L2 => self.run_worker_l2(config),
            Layer::L3 => self.run_worker_l3(config),
        }
    }
}

impl UpdateStateJobTrigger {
    /// This will create new StateTransition jobs for applicable batches
    /// 1. Get the last batch for which state transition happened
    /// 2. Get all the Aggregator jobs after that last batch, that have status as Completed
    /// 3. Sanitize and Trim this list of batches
    /// 4. Create a StateTransition job for doing state transitions for all the batches in this list
    async fn run_worker_l2(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "UpdateStateWorker", "UpdateStateWorker started.");

        // Get the latest StateTransition job
        let latest_job = config.database().get_latest_job_by_type(JobType::StateTransition).await?;

        // Get the aggregator jobs that are completed and are ready to get their StateTransition job created
        let (completed_aggregator_jobs, last_batch_processed_in_last_job) = match latest_job {
            Some(job) => {
                if job.status != JobStatus::Completed {
                    // If we already have a StateTransition job which is not completed, don't start a new job as it can cause issues
                    tracing::warn!(
                        "There's already a pending update state job. Parallel jobs can cause nonce issues or can \
                         completely fail as the update logic needs to be strictly ordered. Returning safely..."
                    );
                    return Ok(());
                }

                // Get the batches from StateTransition job metadata
                let state_metadata: StateUpdateMetadata = job.metadata.specific
                    .try_into()
                    .map_err(|e| {
                        tracing::error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for state transition job");
                        e
                    })?;

                let mut batches_processed = state_metadata.batches_to_settle.clone();
                batches_processed.sort();

                let last_batch_processed = *batches_processed
                    .last()
                    .ok_or_else(|| eyre!("No blocks found in previous state transition job"))?;

                (
                    config
                        .database()
                        .get_jobs_after_internal_id_by_job_type(
                            JobType::Aggregator,
                            JobStatus::Completed,
                            last_batch_processed.to_string(),
                        )
                        .await?,
                    Some(last_batch_processed),
                )
            }
            None => {
                tracing::warn!("No previous state transition job found, fetching latest aggregator job");
                // Get the latest Aggregator job in case no StateTransition job is present
                (
                    config
                        .database()
                        .get_jobs_without_successor(JobType::Aggregator, JobStatus::Completed, JobType::StateTransition)
                        .await?,
                    None,
                )
            }
        };

        let mut batches_to_process: Vec<u64> =
            completed_aggregator_jobs.iter().map(|j| j.internal_id.parse::<u64>().unwrap()).collect();
        batches_to_process.sort();

        // no Aggregator jobs completed after the last settled block
        if batches_to_process.is_empty() {
            tracing::warn!("No Aggregator jobs completed after the last settled block. Returning safely...");
            return Ok(());
        }

        // Verify batch continuity
        match last_batch_processed_in_last_job {
            Some(last_batch) => {
                // Checking if the batch to be processed is exactly one more than the last processed batch
                if batches_to_process[0] != last_batch + 1 {
                    tracing::warn!(
                        "Aggregator job for the block just after the last settled block ({}) is not yet completed. Returning safely...", last_batch
                    );
                    return Ok(());
                }
            }
            None => {
                // if the last processed batch is not there, (i.e., this is the first StateTransition job), check if the batch being processed is equal to 1
                if batches_to_process[0] != 1 {
                    tracing::warn!("Aggregator job for the first batch is not yet completed. Can't proceed with batch {}, Returning safely...", batches_to_process[0]);
                    return Ok(());
                }
            }
        }

        // Sanitize the list of batches to be processed
        let mut batches_to_process = find_successive_batches_in_vector(batches_to_process);
        // Cap the number of batches that can be processed in a single job
        if batches_to_process.len() > 10 {
            batches_to_process = batches_to_process.into_iter().take(10).collect();
        }

        // Prepare state transition metadata
        let mut state_metadata = StateUpdateMetadata {
            batches_to_settle: batches_to_process.clone(),
            last_failed_batch_no: None,
            tx_hashes: Vec::new(),
            ..Default::default()
        };

        // Collect paths from the Aggregator job
        for batch_no in &batches_to_process {
            // Get the aggregator job metadata for the batch
            let aggregator_job = config
                .database()
                .get_job_by_internal_id_and_type(&batch_no.to_string(), &JobType::Aggregator)
                .await?
                .ok_or_else(|| eyre!("SNOS job not found for block {}", batch_no))?;
            let aggregator_metadata: AggregatorMetadata = aggregator_job.metadata.specific.try_into().map_err(|e| {
                tracing::error!(job_id = %aggregator_job.internal_id, error = %e, "Invalid metadata type for Aggregator job");
                e
            })?;

            // Add the snos output path, program output path and blob data path in state transition metadata
            state_metadata.snos_output_paths.push(aggregator_metadata.snos_output_path.clone());
            state_metadata.program_output_paths.push(aggregator_metadata.program_output_path.clone());
            state_metadata.blob_data_paths.push(aggregator_metadata.blob_data_path.clone());
        }

        // Create StateTransition job metadata
        let metadata = JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(state_metadata),
        };

        // Create the state transition job
        let new_job_id = batches_to_process[0].to_string(); // internal_id for StateTransition is the first batch to be processed
        match JobHandlerService::create_job(JobType::StateTransition, new_job_id.clone(), metadata, config.clone())
            .await
        {
            Ok(_) => tracing::info!(job_id = %new_job_id, "Successfully created new state transition job"),
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

    async fn run_worker_l3(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "UpdateStateWorker", "UpdateStateWorker started.");

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

        let mut blocks_to_process = find_successive_blocks_in_vector(blocks_to_process);

        // TODO: Remove this once we have a proper way to handle L3 blocks with use of receipt
        let max_blocks = match config.layer() {
            Layer::L2 => 10,
            Layer::L3 => 1,
        };
        if blocks_to_process.len() >= max_blocks {
            blocks_to_process = blocks_to_process.into_iter().take(max_blocks).collect();
        }

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
/// e.g.: input_vec : [1,2,3,4,7,8,9,11]
/// We will take the first 4 block numbers and send it for processing
pub fn find_successive_blocks_in_vector(block_numbers: Vec<u64>) -> Vec<u64> {
    block_numbers
        .iter()
        .enumerate()
        .take_while(|(index, block_number)| **block_number == (block_numbers[0] + *index as u64))
        .map(|(_, block_number)| *block_number)
        .collect()
}

/// Gets the successive list of blocks from all the blocks processed in previous jobs
/// e.g.: input_vec : [1,2,3,4,7,8,9,11]
/// We will take the first 4 batch numbers and send it for processing
pub fn find_successive_batches_in_vector(batch_numbers: Vec<u64>) -> Vec<u64> {
    batch_numbers
        .iter()
        .enumerate()
        .take_while(|(index, batch_number)| **batch_number == (batch_numbers[0] + *index as u64))
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
