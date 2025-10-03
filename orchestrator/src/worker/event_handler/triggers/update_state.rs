use crate::core::config::Config;
use crate::types::jobs::metadata::{
    AggregatorMetadata, CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, SettlementContext,
    SettlementContextData, SnosMetadata, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::constants::{STATE_UPDATE_MAX_NO_BATCH_PROCESSING, STATE_UPDATE_MAX_NO_BLOCK_PROCESSING};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use color_eyre::eyre::eyre;
use itertools::Itertools;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;
use std::sync::Arc;
use tracing::{error, info, trace, warn};

pub struct UpdateStateJobTrigger;

#[async_trait]
impl JobTrigger for UpdateStateJobTrigger {
    /// This will create new StateTransition jobs for applicable batches
    /// 1. Get the last batch for which state transition happened
    /// 2. Get all the parent jobs after that last block/batch that have status as Completed
    /// 3. Sanitize and Trim this list of batches
    /// 4. Create a StateTransition job for doing state transitions for all the batches in this list
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        trace!(log_type = "starting", "UpdateStateWorker started.");

        // Self-healing: recover any orphaned StateTransition jobs before creating new ones
        if let Err(e) = self.heal_orphaned_jobs(config.clone(), JobType::StateTransition).await {
            error!(error = %e, "Failed to heal orphaned StateTransition jobs, continuing with normal processing");
        }

        let parent_job_type = match config.layer() {
            Layer::L2 => JobType::Aggregator,
            Layer::L3 => JobType::DataSubmission,
        };

        // Get the latest StateTransition job
        let latest_job = config.database().get_latest_job_by_type(JobType::StateTransition).await?;
        // Get the parent jobs that are completed and are ready to get their StateTransition job created
        let (jobs_to_process, last_processed) = match latest_job {
            Some(job) => {
                if job.status != JobStatus::Completed {
                    // If we already have a StateTransition job which is not completed, don't start a new job as it can cause issues
                    warn!(
                        "There's already a pending update state job. Parallel jobs can cause nonce issues or can \
                         completely fail as the update logic needs to be strictly ordered. Returning safely..."
                    );
                    return Ok(());
                }

                // Extract blocks/batches from state transition metadata
                let state_update_metadata: StateUpdateMetadata = job.metadata.specific.try_into().map_err(|e| {
                    error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for state transition job");
                    e
                })?;

                let mut processed = match state_update_metadata.context {
                    SettlementContext::Block(block) => block.to_settle,
                    SettlementContext::Batch(batch) => batch.to_settle,
                };
                processed.sort();

                let last_processed = *processed
                    .last()
                    .ok_or_else(|| eyre!("No blocks/batches found in previous state transition job"))?;

                (
                    config
                        .database()
                        .get_jobs_after_internal_id_by_job_type(
                            parent_job_type,
                            JobStatus::Completed,
                            last_processed.to_string(),
                        )
                        .await?,
                    Some(last_processed),
                )
            }
            None => {
                warn!("No previous state transition job found, fetching latest parent job");
                // Getting the latest parent job in case no latest state update job is present
                (
                    config
                        .database()
                        .get_jobs_without_successor(parent_job_type, JobStatus::Completed, JobType::StateTransition)
                        .await?,
                    None,
                )
            }
        };

        let mut to_process: Vec<u64> = jobs_to_process.iter().map(|j| j.internal_id.parse::<u64>()).try_collect()?;
        to_process.sort();

        // no parent jobs completed after the last settled block
        if to_process.is_empty() {
            warn!("No parent jobs completed after the last settled block/batch. Returning safely...");
            return Ok(());
        }

        // Verify block continuity
        match last_processed {
            Some(last_block) => {
                if to_process[0] != last_block + 1 {
                    warn!(
                        "Parent job for the block/batch just after the last settled block/batch ({}) is not yet completed. Returning safely...", last_block
                    );
                    return Ok(());
                }
            }
            None => {
                match config.layer() {
                    Layer::L2 => {
                        // if the last processed batch is not there, (i.e., this is the first StateTransition job), check if the batch being processed is equal to 1
                        if to_process[0] != 1 {
                            warn!("Aggregator job for the first batch is not yet completed. Can't proceed with batch {}, Returning safely...", to_process[0]);
                            return Ok(());
                        }
                    }
                    Layer::L3 => {
                        // If the last processed block is not there, check if the first being processed is equal to min_block_to_process (default=0)
                        let min_block_to_process = config.service_config().min_block_to_process;
                        if to_process[0] != min_block_to_process {
                            warn!("DA job for the first block is not yet completed. Returning safely...");
                            return Ok(());
                        }
                    }
                }
            }
        }

        // Sanitize the list of blocks/batches to be processed
        let to_process =
            find_successive_items_in_vector(to_process, Some(self.max_items_to_process_in_single_job(config.layer())));

        // Getting settlement context
        let settlement_context = match config.layer() {
            Layer::L2 => {
                SettlementContext::Batch(SettlementContextData { to_settle: to_process.clone(), last_failed: None })
            }
            Layer::L3 => {
                SettlementContext::Block(SettlementContextData { to_settle: to_process.clone(), last_failed: None })
            }
        };

        // Prepare state transition metadata
        let mut state_update_metadata = StateUpdateMetadata { context: settlement_context, ..Default::default() };

        // Collect paths for the following - snos output, program output and blob data
        match config.layer() {
            Layer::L2 => self.collect_paths_l2(&config, &to_process, &mut state_update_metadata).await?,
            Layer::L3 => self.collect_paths_l3(&config, &to_process, &mut state_update_metadata).await?,
        }

        let starknet_version = if let Some(first_job) = jobs_to_process.first() {
            first_job.metadata.common.starknet_version.clone()
        } else {
            None
        };

        // Create StateTransition job metadata, propagating Starknet version from parent jobs
        let metadata = JobMetadata {
            common: CommonMetadata { starknet_version, ..Default::default() },
            specific: JobSpecificMetadata::StateUpdate(state_update_metadata),
        };

        // Create the state transition job
        let new_job_id = to_process[0].to_string();
        match JobHandlerService::create_job(JobType::StateTransition, new_job_id.clone(), metadata, config.clone())
            .await
        {
            Ok(_) => info!(job_id = %new_job_id, "Successfully created new state transition job"),
            Err(e) => {
                error!(job_id = %new_job_id, error = %e, "Failed to create new state transition job");
                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::StateTransition)),
                    KeyValue::new("operation_type", format!("{:?}", "create_job")),
                ];
                ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                return Err(e.into());
            }
        }

        trace!(log_type = "completed", "UpdateStateWorker completed.");
        Ok(())
    }
}

impl UpdateStateJobTrigger {
    async fn collect_paths_l3(
        &self,
        config: &Arc<Config>,
        blocks_to_process: &Vec<u64>,
        state_metadata: &mut StateUpdateMetadata,
    ) -> color_eyre::Result<()> {
        for block_number in blocks_to_process {
            // Get SNOS job paths
            let snos_job = config
                .database()
                .get_job_by_internal_id_and_type(&block_number.to_string(), &JobType::SnosRun)
                .await?
                .ok_or_else(|| eyre!("SNOS job not found for block {}", block_number))?;
            let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().map_err(|e| {
                error!(job_id = %snos_job.internal_id, error = %e, "Invalid metadata type for SNOS job");
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
                error!(job_id = %da_job.internal_id, error = %e, "Invalid metadata type for DA job");
                e
            })?;

            if let Some(blob_path) = &da_metadata.blob_data_path {
                state_metadata.blob_data_paths.push(blob_path.clone());
            }
        }

        Ok(())
    }

    async fn collect_paths_l2(
        &self,
        config: &Arc<Config>,
        batches_to_process: &Vec<u64>,
        state_metadata: &mut StateUpdateMetadata,
    ) -> color_eyre::Result<()> {
        for batch_no in batches_to_process {
            // Get the aggregator job metadata for the batch
            let aggregator_job = config
                .database()
                .get_job_by_internal_id_and_type(&batch_no.to_string(), &JobType::Aggregator)
                .await?
                .ok_or_else(|| eyre!("SNOS job not found for block {}", batch_no))?;
            let aggregator_metadata: AggregatorMetadata = aggregator_job.metadata.specific.try_into().map_err(|e| {
                error!(job_id = %aggregator_job.internal_id, error = %e, "Invalid metadata type for Aggregator job");
                e
            })?;

            // Add the snos output path, program output path and blob data path in state transition metadata
            state_metadata.snos_output_paths.push(aggregator_metadata.snos_output_path.clone());
            state_metadata.program_output_paths.push(aggregator_metadata.program_output_path.clone());
            state_metadata.blob_data_paths.push(aggregator_metadata.blob_data_path.clone());
        }

        Ok(())
    }

    /// Get the maximum number of items to process in a single job
    fn max_items_to_process_in_single_job(&self, layer: &Layer) -> usize {
        match layer {
            Layer::L2 => STATE_UPDATE_MAX_NO_BATCH_PROCESSING,
            Layer::L3 => STATE_UPDATE_MAX_NO_BLOCK_PROCESSING,
        }
    }
}

/// Gets the successive list of blocks from all the blocks processed in previous jobs
/// e.g.: input_vec : [1,2,3,4,7,8,9,11]
/// We will take the first 4 block numbers and send it for processing
pub fn find_successive_items_in_vector(items: Vec<u64>, limit: Option<usize>) -> Vec<u64> {
    items
        .iter()
        .enumerate()
        .take_while(|(index, block_number)| {
            **block_number == (items[0] + *index as u64) && (limit.is_none() || *index < limit.unwrap())
        })
        .map(|(_, block_number)| *block_number)
        .collect()
}

#[cfg(test)]
mod test_update_state_worker_utils {
    use rstest::rstest;

    #[rstest]
    #[case(vec![], Some(3), vec![])]
    #[case(vec![1], None, vec![1])]
    #[case(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], None, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])]
    #[case(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], Some(10), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10])]
    #[case(vec![1, 2, 3, 4, 5], Some(3), vec![1, 2, 3])] // limit smaller than available
    #[case(vec![1, 2, 3], Some(5), vec![1, 2, 3])] // limit larger than available
    fn test_find_successive_items(#[case] input: Vec<u64>, #[case] limit: Option<usize>, #[case] expected: Vec<u64>) {
        let result = super::find_successive_items_in_vector(input, limit);
        assert_eq!(result, expected);
    }
}
