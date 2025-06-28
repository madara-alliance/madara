use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::{eyre, Context};
use opentelemetry::KeyValue;

use crate::core::config::Config;
use crate::types::jobs::job_item::JobItem;
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
    /// Creates state transition jobs by collecting completed DA jobs and building state updates.
    ///
    /// This worker:
    /// 1. Checks for existing pending state transition jobs (prevents parallel execution)
    /// 2. Determines blocks to process based on completed DA jobs and previous state transitions
    /// 3. Validates block continuity to ensure proper ordering
    /// 4. Collects metadata from SNOS and DA jobs for the selected blocks
    /// 5. Creates a state transition job with all necessary paths and metadata
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "UpdateStateWorker", "UpdateStateWorker started.");

        let processing_context = ProcessingContext::new(config.clone()).await?;

        // Check for existing pending jobs first
        if let Some(pending_reason) = processing_context.check_pending_jobs().await? {
            tracing::warn!("{}", pending_reason);
            return Ok(());
        }

        let blocks_to_process = match processing_context.determine_blocks_to_process().await? {
            Some(blocks) => blocks,
            None => {
                // Already logged the reason in determine_blocks_to_process
                return Ok(());
            }
        };

        let state_metadata = self.build_state_metadata(&blocks_to_process, &processing_context).await?;

        self.create_state_transition_job(&blocks_to_process, state_metadata, processing_context.config.clone()).await?;

        tracing::trace!(log_type = "completed", category = "UpdateStateWorker", "UpdateStateWorker completed.");
        Ok(())
    }
}

impl UpdateStateJobTrigger {
    /// Builds complete state metadata by collecting paths from SNOS and DA jobs
    async fn build_state_metadata(
        &self,
        blocks_to_process: &[u64],
        context: &ProcessingContext,
    ) -> color_eyre::Result<StateUpdateMetadata> {
        let mut state_metadata = StateUpdateMetadata {
            blocks_to_settle: blocks_to_process.to_vec(),
            snos_output_paths: Vec::new(),
            program_output_paths: Vec::new(),
            blob_data_paths: Vec::new(),
            last_failed_block_no: None,
            tx_hashes: Vec::new(),
        };

        for &block_number in blocks_to_process {
            let snos_paths = self.collect_snos_paths(block_number, context).await?;
            let da_paths = self.collect_da_paths(block_number, context).await?;

            state_metadata.snos_output_paths.extend(snos_paths.snos_output_paths);
            state_metadata.program_output_paths.extend(snos_paths.program_output_paths);
            state_metadata.blob_data_paths.extend(da_paths.blob_data_paths);
        }

        Ok(state_metadata)
    }

    /// Collects SNOS-related paths for a specific block
    async fn collect_snos_paths(
        &self,
        block_number: u64,
        context: &ProcessingContext,
    ) -> color_eyre::Result<SnosPaths> {
        let snos_job = context.get_snos_job(block_number).await?;
        let snos_metadata = self.extract_snos_metadata(&snos_job)?;

        Ok(SnosPaths {
            snos_output_paths: snos_metadata.snos_output_path.into_iter().collect(),
            program_output_paths: snos_metadata.program_output_path.into_iter().collect(),
        })
    }

    /// Collects DA-related paths for a specific block
    async fn collect_da_paths(&self, block_number: u64, context: &ProcessingContext) -> color_eyre::Result<DaPaths> {
        let da_job = context.get_da_job(block_number).await?;
        let da_metadata = self.extract_da_metadata(&da_job)?;

        Ok(DaPaths { blob_data_paths: da_metadata.blob_data_path.into_iter().collect() })
    }

    /// Extracts SNOS metadata from a job with error handling
    fn extract_snos_metadata(&self, snos_job: &JobItem) -> color_eyre::Result<SnosMetadata> {
        snos_job
            .metadata
            .specific
            .clone()
            .try_into()
            .map_err(|e| {
                tracing::error!(
                    job_id = %snos_job.internal_id,
                    error = %e,
                    "Invalid metadata type for SNOS job"
                );
                e
            })
            .context("Unable to extract SNOS metadata")
    }

    /// Extracts DA metadata from a job with error handling
    fn extract_da_metadata(&self, da_job: &JobItem) -> color_eyre::Result<DaMetadata> {
        da_job
            .metadata
            .specific
            .clone()
            .try_into()
            .map_err(|e| {
                tracing::error!(
                    job_id = %da_job.internal_id,
                    error = %e,
                    "Invalid metadata type for DA job"
                );
                e
            })
            .context("Unable to extract SNOS metadata")
    }

    /// Creates the actual state transition job
    async fn create_state_transition_job(
        &self,
        blocks_to_process: &[u64],
        state_metadata: StateUpdateMetadata,
        config: Arc<Config>,
    ) -> color_eyre::Result<()> {
        let metadata = JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(state_metadata),
        };

        let new_job_id = blocks_to_process[0].to_string();

        tracing::info!(
            job_id = %new_job_id,
            blocks_count = blocks_to_process.len(),
            blocks = ?blocks_to_process,
            "Creating state transition job"
        );

        JobHandlerService::create_job(JobType::StateTransition, new_job_id.clone(), metadata, config).await.map_err(
            |e| {
                tracing::error!(
                    job_id = %new_job_id,
                    error = %e,
                    "Failed to create state transition job"
                );

                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::StateTransition)),
                    KeyValue::new("operation_type", "create_job"),
                ];
                ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);

                e
            },
        )?;

        tracing::info!(job_id = %new_job_id, "Successfully created state transition job");
        Ok(())
    }
}

/// Context for processing state transitions, containing shared data and configuration
struct ProcessingContext {
    config: Arc<Config>,
}

impl ProcessingContext {
    /// Creates a new processing context
    async fn new(config: Arc<Config>) -> color_eyre::Result<Self> {
        Ok(Self { config })
    }

    /// Checks if there are any pending state transition jobs that would prevent new job creation
    async fn check_pending_jobs(&self) -> color_eyre::Result<Option<String>> {
        let latest_job = self.config.database().get_latest_job_by_type(JobType::StateTransition).await?;

        if let Some(job) = latest_job {
            if job.status != JobStatus::Completed {
                return Ok(Some(
                    "There's already a pending update state job. Parallel jobs can cause nonce issues or can \
                     completely fail as the update logic needs to be strictly ordered. Returning safely..."
                        .to_string(),
                ));
            }
        }

        Ok(None)
    }

    /// Determines which blocks should be processed based on completed DA jobs and previous state transitions
    /// Determines which blocks should be processed based on completed DA jobs and previous state transitions
    async fn determine_blocks_to_process(&self) -> color_eyre::Result<Option<Vec<u64>>> {
        let latest_job = self.config.database().get_latest_job_by_type(JobType::StateTransition).await?;

        let earliest_failed_block = self.config.database().get_earliest_failed_block_number().await?;

        let (completed_da_jobs, last_block_processed) = match latest_job {
            Some(job) => {
                let last_block = self.get_last_processed_block(&job).await?;
                let da_jobs = self.get_da_jobs_after_block(last_block).await?;
                (da_jobs, Some(last_block))
            }
            None => {
                tracing::warn!("No previous state transition job found, fetching latest data submission job");
                let da_jobs = self.get_all_unprocessed_da_jobs().await?;
                (da_jobs, None)
            }
        };

        let mut blocks_to_process = self.extract_block_numbers(&completed_da_jobs)?;

        if blocks_to_process.is_empty() {
            tracing::warn!("No DA jobs completed after the last settled block. Returning safely...");
            return Ok(None);
        }

        // Filter out blocks that are at or after the earliest failed block (if it exists)
        if let Some(failed_block) = earliest_failed_block {
            blocks_to_process.retain(|&block| block < failed_block);

            if blocks_to_process.is_empty() {
                tracing::warn!(
                    "All blocks to process are at or after earliest failed block {}. Returning safely...",
                    failed_block
                );
                return Ok(None);
            }
        }

        blocks_to_process.sort();

        // Validate block continuity
        if !self.validate_block_continuity(&blocks_to_process, last_block_processed).await? {
            return Ok(None);
        }

        // Find successive blocks and limit to maximum batch size
        let successive_blocks = find_successive_blocks_in_vector(blocks_to_process);
        let final_blocks = self.limit_batch_size(successive_blocks);

        Ok(Some(final_blocks))
    }

    /// Extracts the last processed block from a state transition job
    async fn get_last_processed_block(&self, job: &JobItem) -> color_eyre::Result<u64> {
        // TODO: This is what I don't like, we know we are working with StateTransition Job, still we have to treat it like a general job.
        let state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for state transition job");
            e
        })?;

        let mut blocks_processed = state_metadata.blocks_to_settle;
        blocks_processed.sort();

        blocks_processed.last().copied().ok_or_else(|| eyre!("No blocks found in previous state transition job"))
    }

    /// Gets DA jobs processed after a specific block number
    async fn get_da_jobs_after_block(&self, last_block: u64) -> color_eyre::Result<Vec<JobItem>> {
        self.config
            .database()
            .get_jobs_after_internal_id_by_job_type(
                JobType::DataSubmission,
                JobStatus::Completed,
                last_block.to_string(),
            )
            .await
            .context("Unable to get DA jobs after specified block")
    }

    /// Gets all DA jobs that don't have state transition successors
    async fn get_all_unprocessed_da_jobs(&self) -> color_eyre::Result<Vec<JobItem>> {
        self.config
            .database()
            .get_jobs_without_successor(JobType::DataSubmission, JobStatus::Completed, JobType::StateTransition)
            .await
            .context("Unable to get all Unprocessed DA jobs")
    }

    /// Extracts block numbers from DA jobs
    fn extract_block_numbers(&self, da_jobs: &[JobItem]) -> color_eyre::Result<Vec<u64>> {
        da_jobs
            .iter()
            .map(|job| {
                job.internal_id.parse::<u64>().map_err(|e| {
                    tracing::error!(job_id = %job.internal_id, error = %e, "Failed to parse job ID as block number");
                    eyre!("Invalid block number in job ID: {}", job.internal_id)
                })
            })
            .collect()
    }

    /// Validates that blocks are continuous from the expected starting point
    async fn validate_block_continuity(
        &self,
        blocks: &[u64],
        last_block_processed: Option<u64>,
    ) -> color_eyre::Result<bool> {
        let expected_first_block = match last_block_processed {
            Some(last_block) => {
                if blocks[0] != last_block + 1 {
                    tracing::warn!(
                        "DA job for the block just after the last settled block is not yet completed. Returning safely..."
                    );
                    return Ok(false);
                }
                last_block + 1
            }
            None => {
                let min_block = self.config.service_config().min_block_to_process;
                if blocks[0] != min_block {
                    tracing::warn!("DA job for the first block is not yet completed. Returning safely...");
                    return Ok(false);
                }
                min_block
            }
        };

        Ok(blocks[0] == expected_first_block)
    }

    /// Limits the batch size to maximum allowed blocks
    fn limit_batch_size(&self, mut blocks: Vec<u64>) -> Vec<u64> {
        const MAX_BATCH_SIZE: usize = 10;

        if blocks.len() > MAX_BATCH_SIZE {
            blocks.truncate(MAX_BATCH_SIZE);
            tracing::info!("Limited batch size to {} blocks for processing efficiency", MAX_BATCH_SIZE);
        }

        blocks
    }

    /// Gets a SNOS job for a specific block number
    async fn get_snos_job(&self, block_number: u64) -> color_eyre::Result<JobItem> {
        self.config
            .database()
            .get_job_by_internal_id_and_type(&block_number.to_string(), &JobType::SnosRun)
            .await?
            .ok_or_else(|| eyre!("SNOS job not found for block {}", block_number))
    }

    /// Gets a DA job for a specific block number
    async fn get_da_job(&self, block_number: u64) -> color_eyre::Result<JobItem> {
        self.config
            .database()
            .get_job_by_internal_id_and_type(&block_number.to_string(), &JobType::DataSubmission)
            .await?
            .ok_or_else(|| eyre!("DA job not found for block {}", block_number))
    }
}

/// Paths collected from SNOS jobs
struct SnosPaths {
    snos_output_paths: Vec<String>,
    program_output_paths: Vec<String>,
}

/// Paths collected from DA jobs
struct DaPaths {
    blob_data_paths: Vec<String>,
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
