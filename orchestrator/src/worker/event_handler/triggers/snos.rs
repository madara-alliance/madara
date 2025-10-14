use crate::core::config::Config;
use crate::types::batch::{SnosBatchStatus, SnosBatchUpdates};
use crate::types::constant::{
    CAIRO_PIE_FILE_NAME, ON_CHAIN_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::types::JobType;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use color_eyre::eyre::Result;
use opentelemetry::KeyValue;
use std::sync::Arc;
use tracing::{error, info, trace, warn};

/// Triggers the creation of SNOS (Starknet Network Operating System) jobs.
///
/// This component is responsible for:
/// - Determining which blocks need SNOS processing
/// - Managing job creation within concurrency limits
/// - Ensuring proper ordering and dependencies between jobs
pub struct SnosJobTrigger;

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
        trace!(log_type = "starting", category = "SnosRunWorker", "SnosRunWorker started.");

        // Self-healing: recover any orphaned SNOS jobs before creating new ones
        if let Err(e) = self.heal_orphaned_jobs(config.clone(), JobType::SnosRun).await {
            error!(error = %e, "Failed to heal orphaned SNOS jobs, continuing with normal processing");
        }

        // Get all snos batches that are closed but don't have a SnosRun job created yet
        for snos_batch in config.database().get_snos_batches_without_jobs(SnosBatchStatus::Closed).await? {
            // Create DA metadata
            let snos_metadata = create_job_metadata(
                snos_batch.start_block,
                snos_batch.end_block,
                config.snos_config().snos_full_output,
            );

            match JobHandlerService::create_job(
                JobType::SnosRun,
                snos_batch.snos_batch_id.clone().to_string(), /* changing mapping here snos_batch_id => internal_id for snos and then eventually proving jobs*/
                snos_metadata,
                config.clone(),
            )
                .await
            {
                Ok(_) => {
                    info!(batch_id = %snos_batch.snos_batch_id,"Successfully created new snos job");
                    config.database().update_or_create_snos_batch(&snos_batch, &SnosBatchUpdates {end_block: None, status: Some(SnosBatchStatus::SnosJobCreated)}).await?;
                },
                Err(e) => {
                    warn!(
                        batch_id = %snos_batch.snos_batch_id,
                        error = %e,
                        "Failed to create new snos job"
                    );
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::SnosRun)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        trace!(log_type = "completed", category = "SnosWorker", "SnosWorker completed.");
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
pub async fn fetch_block_starknet_version(config: &Arc<Config>, block_number: u64) -> Result<String> {
    use color_eyre::eyre::{eyre, WrapErr};
    use starknet::core::types::BlockId;
    use starknet::providers::Provider;

    let provider = config.madara_client();
    tracing::debug!("Fetching block header for block {} to extract Starknet version", block_number);

    // Fetch block with transaction hashes (lighter than full txs)
    let block = provider
        .get_block_with_tx_hashes(BlockId::Number(block_number))
        .await
        .wrap_err(format!("Failed to fetch block {} from sequencer", block_number))?;

    let starknet_version = match block {
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::Block(block) => block.starknet_version,
        starknet::core::types::MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            return Err(eyre!(
                "Block {} is still pending/pre-confirmed, cannot determine final Starknet version",
                block_number
            ));
        }
    };

    Ok(starknet_version)
}

// create_job_metadata is a helper function to create job metadata for a given block number and layer
// set full_output to true if layer is L3, false otherwise
fn create_job_metadata(start_block: u64, end_block: u64, full_output: bool) -> JobMetadata {
    JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Snos(SnosMetadata {
            start_block,
            end_block,
            num_blocks: end_block - start_block + 1,
            full_output,
            cairo_pie_path: Some(format!("{}/{}", start_block, CAIRO_PIE_FILE_NAME)),
            on_chain_data_path: Some(format!("{}/{}", start_block, ON_CHAIN_DATA_FILE_NAME)),
            snos_output_path: Some(format!("{}/{}", start_block, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", start_block, PROGRAM_OUTPUT_FILE_NAME)),
            ..Default::default()
        }),
    }
}
