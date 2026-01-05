use crate::core::config::{Config, StarknetVersion};
use crate::types::batch::SnosBatchStatus;
use crate::types::constant::{
    CAIRO_PIE_FILE_NAME, ON_CHAIN_DATA_FILE_NAME, ORCHESTRATOR_VERSION, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use color_eyre::eyre::{eyre, Result};
use opentelemetry::KeyValue;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Triggers the creation of SNOS (Starknet Network Operating System) jobs.
///
/// This component is responsible for:
/// - Determining which blocks need SNOS processing
/// - Managing job creation within concurrency limits
/// - Ensuring proper ordering and dependencies between jobs
pub struct SnosJobTrigger;

/// Calculates the maximum number of SNOS jobs to create based on the current buffer state.
///
/// The buffer size represents the target number of jobs to maintain in the processing pipeline.
/// This function calculates how many new jobs can be created to reach that target.
///
/// # Arguments
/// * `latest_job_internal_id` - The internal ID of the most recently created SNOS job, if any
/// * `oldest_incomplete_internal_id` - The internal ID of the oldest non-completed SNOS job, if any
/// * `buffer_size` - The target buffer size from configuration
///
/// # Returns
/// The number of new jobs that can be created without exceeding the buffer size
///
/// # Logic
/// - If no jobs exist at all: can create up to `buffer_size` jobs
/// - If jobs exist but all are completed: can create up to `buffer_size` jobs
/// - Otherwise: create enough jobs to fill the buffer (buffer_size - current_buffer_count)
fn calculate_max_snos_jobs_to_create(
    latest_job_internal_id: Option<u64>,
    oldest_incomplete_internal_id: Option<u64>,
    buffer_size: u64,
) -> u64 {
    match (latest_job_internal_id, oldest_incomplete_internal_id) {
        (Some(latest), Some(oldest_incomplete)) => {
            let num_jobs_in_buffer = latest - oldest_incomplete + 1;
            buffer_size.saturating_sub(num_jobs_in_buffer)
        }
        // No jobs exist, or all jobs are completed - can create full buffer
        _ => buffer_size,
    }
}

#[async_trait]
impl JobTrigger for SnosJobTrigger {
    /// Main entry point for SNOS job creation workflow.
    ///
    /// This method orchestrates the entire job scheduling process:
    /// 1. Calculates processing boundaries based on sequencer state and completed jobs
    /// 2. Determines available concurrency slots
    /// 3. Schedules blocks for processing within slot constraints
    /// 4. Creates the actual jobs in the database
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database, client, and service settings
    ///
    /// # Returns
    /// * `Result<()>` - Success or error from the job creation process
    ///
    /// # Behavior
    /// - Returns early if no slots are available for new jobs
    /// - Respects concurrency limits defined in service configuration
    /// - Processes blocks in order while filling available slots efficiently
    async fn run_worker(&self, config: Arc<Config>) -> Result<()> {
        // Self-healing: recover any orphaned SNOS jobs before creating new ones
        if let Err(e) = self.heal_orphaned_jobs(config.clone(), JobType::SnosRun).await {
            error!(error = %e, "Failed to heal orphaned SNOS jobs, continuing with normal processing");
        }

        // Get the target buffer size from config (configurable via env, defaults to 50)
        let snos_job_buffer_size = config.service_config().snos_job_buffer_size;

        // Get latest SNOS job internal ID (if any)
        let latest_job_internal_id =
            if let Some(latest_job) = config.database().get_latest_job_by_type(JobType::SnosRun).await? {
                Some(latest_job.internal_id)
            } else {
                None
            };

        // Get oldest incomplete SNOS job internal ID (if any)
        let oldest_incomplete_internal_id = if let Some(oldest_incomplete_job) = config
            .database()
            .get_oldest_job_by_type_excluding_statuses(JobType::SnosRun, vec![JobStatus::Completed])
            .await?
        {
            Some(oldest_incomplete_job.internal_id)
        } else {
            None
        };

        // Calculate the maximum number of SNOS jobs to create
        let max_jobs_to_create = calculate_max_snos_jobs_to_create(
            latest_job_internal_id,
            oldest_incomplete_internal_id,
            snos_job_buffer_size,
        );

        // Early return if buffer is full - no jobs to create
        if max_jobs_to_create == 0 {
            debug!(buffer_size = %snos_job_buffer_size, "SNOS job buffer is full, no new jobs to create. Returning safely.");
            return Ok(());
        }

        info!("Creating max {} {:?} jobs", max_jobs_to_create, JobType::SnosRun);

        // Get all snos batches that are closed but don't have a SnosRun job created yet
        for snos_batch in config
            .database()
            .get_snos_batches_without_jobs(SnosBatchStatus::Closed, max_jobs_to_create, Some(ORCHESTRATOR_VERSION))
            .await?
        {
            // Create SNOS job metadata
            let snos_metadata = create_job_metadata(
                snos_batch.index,
                snos_batch.start_block,
                snos_batch.end_block,
                config.snos_config().snos_full_output,
            );

            // Create a new job and add it to the queue
            match JobHandlerService::create_job(
                JobType::SnosRun,
                snos_batch.index, /* changing mapping here snos_batch_id => internal_id for snos and then eventually proving jobs*/
                snos_metadata,
                config.clone(),
            )
                .await
            {
                Ok(_) => {
                    // TODO(prakhar,16/12/2025): Update SNOS batch status to SnosJobCreated
                }
                Err(e) => {
                    error!(error = %e,"Failed to create new {:?} job for {}", JobType::SnosRun, snos_batch.index);
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::SnosRun)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }
}

/// Fetches the Starknet protocol version for a specific block from the sequencer.
///
/// This function queries the Madara client to retrieve the complete block header,
/// which contains the `starknet_version` field indicating which Starknet protocol
/// version was used when the block was created.
///
/// # Arguments
/// * `config` - Application configuration containing the Madara client
/// * `block_number` - The block number to query
///
/// # Returns
/// * `Result<String>` - The Starknet version string (e.g., "0.13.2") or error
///
/// # Errors
/// - Network connectivity issues with the sequencer
/// - Block not found
/// - Missing starknet_version field in block header
pub async fn fetch_block_starknet_version(
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    block_number: u64,
) -> Result<StarknetVersion> {
    use starknet::core::types::BlockId;
    use starknet::providers::Provider;

    debug!("Fetching block header for block {} to extract Starknet version", block_number);

    // Fetch block with transaction hashes (lighter than full txs)
    let block = provider
        .get_block_with_tx_hashes(BlockId::Number(block_number))
        .await
        .map_err(|e| eyre!("Failed to fetch block {} from sequencer: {}", block_number, e))?;

    let starknet_version = match block {
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::Block(block) => block.starknet_version,
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            return Err(eyre!("Block {} is still pending, cannot determine final Starknet version", block_number));
        }
    };

    StarknetVersion::from_str(&starknet_version).map_err(|e| eyre!(e))
}

// create_job_metadata is a helper function to create job metadata for a given block number and layer
// set full_output to true if layer is L2, false otherwise
fn create_job_metadata(snos_batch_id: u64, start_block: u64, end_block: u64, full_output: bool) -> JobMetadata {
    JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Snos(SnosMetadata {
            snos_batch_index: snos_batch_id,
            start_block,
            end_block,
            num_blocks: end_block - start_block + 1,
            full_output,
            cairo_pie_path: Some(format!("{}/{}", snos_batch_id, CAIRO_PIE_FILE_NAME)),
            on_chain_data_path: Some(format!("{}/{}", snos_batch_id, ON_CHAIN_DATA_FILE_NAME)),
            snos_output_path: Some(format!("{}/{}", snos_batch_id, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", snos_batch_id, PROGRAM_OUTPUT_FILE_NAME)),
            ..Default::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_max_snos_jobs_no_jobs_exist() {
        // No jobs exist at all - can create full buffer
        let result = calculate_max_snos_jobs_to_create(None, None, 50);
        assert_eq!(result, 50);
    }

    #[test]
    fn test_calculate_max_snos_jobs_all_completed() {
        // Latest job exists but no incomplete jobs (all completed)
        let result = calculate_max_snos_jobs_to_create(Some(100), None, 50);
        assert_eq!(result, 50);
    }

    #[test]
    fn test_calculate_max_snos_jobs_buffer_partially_full() {
        // Buffer has 10 jobs (latest=110, oldest_incomplete=101)
        // Buffer size is 50, so can create 40 more
        let result = calculate_max_snos_jobs_to_create(Some(110), Some(101), 50);
        assert_eq!(result, 40);
    }

    #[test]
    fn test_calculate_max_snos_jobs_buffer_full() {
        // Buffer has 50 jobs (latest=150, oldest_incomplete=101)
        // Buffer size is 50, so can create 0 more
        let result = calculate_max_snos_jobs_to_create(Some(150), Some(101), 50);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_calculate_max_snos_jobs_buffer_overfull() {
        // Buffer has 60 jobs (latest=160, oldest_incomplete=101)
        // Buffer size is 50, saturating_sub should return 0
        let result = calculate_max_snos_jobs_to_create(Some(160), Some(101), 50);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_calculate_max_snos_jobs_single_incomplete_job() {
        // Only one incomplete job exists (latest=oldest=100)
        // Buffer size is 50, so can create 49 more
        let result = calculate_max_snos_jobs_to_create(Some(100), Some(100), 50);
        assert_eq!(result, 49);
    }
}
