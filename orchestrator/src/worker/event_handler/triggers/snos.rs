use crate::core::config::Config;
use crate::types::constant::{
    is_starknet_version_supported, CAIRO_PIE_FILE_NAME, ON_CHAIN_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME,
    SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SnosMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use color_eyre::eyre::{eyre, Result, WrapErr};
use num_traits::ToPrimitive;
use opentelemetry::KeyValue;
use starknet::providers::Provider;
use std::cmp::{max, min};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Triggers the creation of SNOS (Starknet Network Operating System) jobs.
///
/// This component is responsible for:
/// - Determining which blocks need SNOS processing
/// - Managing job creation within concurrency limits
/// - Ensuring proper ordering and dependencies between jobs
pub struct SnosJobTrigger;

/// Represents the boundaries for block processing.
///
/// These bounds define the range of blocks that can be processed:
/// - `block_n_min`: The minimum block number to consider (based on state updates)
/// - `block_n_completed`: The highest block number that has completed SNOS processing (if any)
/// - `block_n_max`: The maximum block number to process (from sequencer or config limit)
#[derive(Debug)]
struct ProcessingBounds {
    /// Minimum block number to process, derived from completed state updates
    block_n_min: u64,
    /// Latest block number that has completed SNOS processing (None if no jobs completed)
    block_n_completed: Option<u64>,
    /// Maximum block number to process, limited by sequencer or configuration
    block_n_max: u64,
}

/// Context for scheduling SNOS jobs, containing processing state and constraints.
///
/// This structure encapsulates:
/// - Processing boundaries
/// - Available concurrency slots
/// - Blocks selected for processing
#[derive(Debug)]
struct JobSchedulingContext {
    /// The calculated processing boundaries
    bounds: ProcessingBounds,
    /// Number of job slots available for new job creation
    available_slots: u64,
    /// Collection of block numbers selected for job creation
    blocks_to_process: Vec<u64>,
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

        let bounds = self.calculate_processing_bounds(&config).await?;
        let mut context = self.initialize_scheduling_context(&config, bounds).await?;

        if context.available_slots == 0 {
            warn!("All slots occupied by pre-existing jobs, skipping SNOS job creation!");
            return Ok(());
        }

        self.schedule_jobs_for_processing(&config, &mut context).await?;
        self.create_scheduled_jobs(&config, context.blocks_to_process).await?;

        Ok(())
    }
}

impl SnosJobTrigger {
    /// Calculates the processing boundaries based on sequencer state and completed jobs.
    ///
    /// This method determines the valid range of blocks that can be processed by analyzing:
    /// - Latest block from the sequencer (upper bound)
    /// - Configuration limits (min/max block constraints)
    /// - Latest completed SNOS job (progress tracking)
    /// - Latest completed state update job (dependency requirement)
    ///
    /// # Processing Logic
    /// - `block_n_min`: Max of (latest state update block, configured minimum)
    ///   - State updates must complete before SNOS processing
    ///   - Respects configured minimum processing boundary
    /// - `block_n_completed`: Latest completed SNOS block (for gap filling)
    /// - `block_n_max`: Min of (sequencer latest, configured maximum)
    ///   - Cannot process blocks that don't exist yet
    ///   - Respects configured maximum processing boundary
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database and client access
    ///
    /// # Returns
    /// * `Result<ProcessingBounds>` - Calculated boundaries or error
    async fn calculate_processing_bounds(&self, config: &Arc<Config>) -> Result<ProcessingBounds> {
        let latest_sequencer_block = self.fetch_latest_sequencer_block(config).await?;
        let service_config = config.service_config();

        let latest_snos_completed = self.get_latest_completed_snos_block(config).await?;
        let latest_su_completed = self.get_latest_completed_state_update_block(config).await?;

        let block_n_min = latest_su_completed
            .map(|block| max(block, service_config.min_block_to_process))
            .unwrap_or(service_config.min_block_to_process);

        let block_n_max = service_config
            .max_block_to_process
            .map(|bound| min(latest_sequencer_block, bound))
            .unwrap_or(latest_sequencer_block);

        Ok(ProcessingBounds { block_n_min, block_n_completed: latest_snos_completed, block_n_max })
    }

    /// Initializes the job scheduling context with available concurrency slots.
    ///
    /// This method sets up the scheduling context by:
    /// - Calculating available slots based on configuration and existing jobs
    /// - Counting pending/created jobs that consume slots
    /// - Preparing the context for job scheduling decisions
    ///
    /// # Slot Calculation
    /// Available slots = Max concurrent jobs - (Pending jobs + Created jobs)
    /// - Pending jobs: Jobs waiting to be processed or retried
    /// - Created jobs: Jobs created but not yet started
    /// - Uses saturating subtraction to prevent underflow
    ///
    /// # Arguments
    /// * `config` - Application configuration
    /// * `bounds` - Previously calculated processing boundaries
    ///
    /// # Returns
    /// * `Result<JobSchedulingContext>` - Initialized context or error
    async fn initialize_scheduling_context(
        &self,
        config: &Arc<Config>,
        bounds: ProcessingBounds,
    ) -> Result<JobSchedulingContext> {
        let service_config = config.service_config();

        let max_slots = service_config.max_concurrent_created_snos_jobs;
        let pending_jobs_count = self.count_pending_snos_jobs(config, max_slots.to_i64()).await?;
        let available_slots = max_slots.saturating_sub(pending_jobs_count);

        Ok(JobSchedulingContext { bounds, available_slots, blocks_to_process: Vec::new() })
    }

    /// Schedules jobs for processing based on the current context and processing strategy.
    ///
    /// This method implements a two-phase scheduling strategy:
    ///
    /// # Phase 1: Initial Jobs (when block_n_completed is None)
    /// If no SNOS jobs have completed yet:
    /// - Process all missing blocks in range [block_n_min, block_n_max]
    /// - This handles the bootstrap case for new deployments
    ///
    /// # Phase 2: Gap Filling and Forward Progress
    /// When SNOS jobs have completed previously:
    /// - **First Half**: Fill gaps in [block_n_min, block_n_completed]
    ///   - Handles cases where previous jobs failed or were skipped
    ///   - Ensures continuity in processed blocks
    /// - **Second Half**: Process new blocks in [block_n_completed, block_n_max]
    ///   - Advances processing frontier forward
    ///   - Handles newly available blocks from sequencer
    ///
    /// # Slot Management
    /// - Respects available slot limits throughout scheduling
    /// - Stops scheduling when slots are exhausted
    /// - Prioritizes gap filling over forward progress
    ///
    /// # Arguments
    /// * `config` - Application configuration
    /// * `context` - Mutable scheduling context (slots and blocks updated)
    ///
    /// # Returns
    /// * `Result<()>` - Success or scheduling error
    async fn schedule_jobs_for_processing(
        &self,
        config: &Arc<Config>,
        context: &mut JobSchedulingContext,
    ) -> Result<()> {
        // Handle case where no SNOS jobs have completed yet
        if context.bounds.block_n_completed.is_none() {
            return self.schedule_initial_jobs(config, context).await;
        }

        let block_n_completed = context.bounds.block_n_completed.unwrap();

        // Schedule jobs for the first half (block_n_min to block_n_completed)
        self.schedule_jobs_for_range(config, context, context.bounds.block_n_min, block_n_completed, "first half")
            .await?;

        if context.available_slots == 0 {
            return Ok(());
        }

        // Schedule jobs for the second half (block_n_completed to block_n_max)
        self.schedule_jobs_for_range(config, context, block_n_completed, context.bounds.block_n_max, "second half")
            .await?;

        Ok(())
    }

    /// Schedules initial jobs when no SNOS jobs have completed yet.
    ///
    /// This handles the bootstrap scenario where:
    /// - No SNOS jobs have completed successfully (block_n_completed is None)
    /// - All blocks in the valid range need to be considered for processing
    /// - Missing blocks are identified and scheduled within slot limits
    ///
    /// # Use Cases
    /// - Fresh deployment with no processing history
    /// - Recovery from complete job failure scenarios
    /// - Initial processing of historical blocks
    ///
    /// # Arguments
    /// * `config` - Application configuration
    /// * `context` - Mutable scheduling context to update with selected blocks
    ///
    /// # Returns
    /// * `Result<()>` - Success or error from block selection
    async fn schedule_initial_jobs(&self, config: &Arc<Config>, context: &mut JobSchedulingContext) -> Result<()> {
        let candidate_blocks = self
            .get_missing_blocks_in_range(
                config,
                context.bounds.block_n_min,
                context.bounds.block_n_max,
                context.available_slots,
            )
            .await?;

        context.blocks_to_process.extend(candidate_blocks);
        Ok(())
    }

    /// Schedules jobs for a specific block range if processing conditions are met.
    ///
    /// This method handles range-specific job scheduling with several safety checks:
    ///
    /// # Range Validation Cases
    /// - **Invalid Range** (start > end): Skip processing, log debug message
    /// - **Empty Range** (start == end == 0): Skip processing (nothing to do)
    /// - **Valid Range**: Proceed with job scheduling
    ///
    /// # Processing Logic
    /// 1. Validate range boundaries and skip if invalid
    /// 2. Query database for missing blocks in the range
    /// 3. Respect slot limits when selecting blocks
    /// 4. Update context with selected blocks and remaining slots
    ///
    /// # Arguments
    /// * `config` - Application configuration
    /// * `context` - Mutable scheduling context (updated with blocks and slots)
    /// * `start` - Starting block number (inclusive)
    /// * `end` - Ending block number (inclusive)
    /// * `range_name` - Human-readable name for logging ("first half", "second half")
    ///
    /// # Returns
    /// * `Result<()>` - Success or error from block selection
    async fn schedule_jobs_for_range(
        &self,
        config: &Arc<Config>,
        context: &mut JobSchedulingContext,
        start: u64,
        end: u64,
        range_name: &str,
    ) -> Result<()> {
        // Skip if the range is invalid or empty
        if start >= end || end == 0 {
            debug!("Skipping {} range: start={}, end={}", range_name, start, end);
            return Ok(());
        }

        let missing_blocks = self.get_missing_blocks_in_range(config, start, end, context.available_slots).await?;

        context.available_slots = context.available_slots.saturating_sub(missing_blocks.len() as u64);
        context.blocks_to_process.extend(missing_blocks);

        Ok(())
    }

    /// Creates the scheduled SNOS jobs in the database.
    ///
    /// This method finalizes the job creation process by:
    /// - Validating that blocks were selected for processing
    /// - Logging job creation details for observability
    /// - Delegating to the job creation implementation
    ///
    /// # Behavior
    /// - Returns early if no blocks are selected (avoids unnecessary database calls)
    /// - Logs the number and list of blocks being processed
    /// - Calls the external `create_jobs_snos` function to perform actual creation
    ///
    /// # Arguments
    /// * `config` - Application configuration (passed to job creation function)
    /// * `blocks` - Vector of block numbers to create jobs for
    ///
    /// # Returns
    /// * `Result<()>` - Success or error from job creation process
    async fn create_scheduled_jobs(&self, config: &Arc<Config>, blocks: Vec<u64>) -> Result<()> {
        if blocks.is_empty() {
            info!("No blocks to process, skipping job creation");
            return Ok(());
        }

        info!("Creating SNOS jobs for {} blocks: {:?}", blocks.len(), blocks);
        create_jobs_snos(config.clone(), blocks).await
    }

    // Helper methods for fetching data

    /// Fetches the latest block number from the sequencer.
    ///
    /// This method queries the Madara client to get the most recent block number
    /// that has been created by the sequencer. This represents the upper bound
    /// of blocks that could potentially be processed.
    ///
    /// # Arguments
    /// * `config` - Application configuration containing the Madara client
    ///
    /// # Returns
    /// * `Result<u64>` - Latest block number or network/client error
    ///
    /// # Errors
    /// - Network connectivity issues with the sequencer
    /// - Client configuration problems
    /// - Sequencer unavailability
    async fn fetch_latest_sequencer_block(&self, config: &Arc<Config>) -> Result<u64> {
        let provider = config.madara_client();
        debug!("Fetching latest sequencer block");
        provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")
    }

    /// Retrieves the latest block number that has completed SNOS processing.
    ///
    /// This method queries the database for the most recent successfully completed
    /// SNOS job to determine processing progress. The result is used as the
    /// `block_n_completed` for gap detection and scheduling decisions.
    ///
    /// # Return Cases
    /// - `None`: No SNOS jobs have completed successfully yet
    /// - `Some(block_number)`: The highest block number with completed SNOS processing
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database access
    ///
    /// # Returns
    /// * `Result<Option<u64>>` - Latest completed block number or database error
    ///
    /// # Panics
    /// - If database returns SNOS job with non-SNOS metadata (data integrity issue)
    async fn get_latest_completed_snos_block(&self, config: &Arc<Config>) -> Result<Option<u64>> {
        let db = config.database();
        match db.get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed).await? {
            None => Ok(None),
            Some(job_item) => match job_item.metadata.specific {
                JobSpecificMetadata::Snos(metadata) => Ok(Some(metadata.block_number)),
                _ => panic!("Unexpected metadata type for SNOS job"),
            },
        }
    }

    /// Retrieves the latest block number from completed state update jobs.
    ///
    /// State update jobs must complete before SNOS processing can begin for those blocks.
    /// This method finds the highest block number that has completed state updates,
    /// which determines the minimum processing boundary for SNOS jobs.
    ///
    /// # Processing Logic
    /// - Queries for the latest completed StateTransition job
    /// - Extracts the maximum block number from `blocks_to_settle`
    /// - State updates can process multiple blocks in a single job
    ///
    /// # Return Cases
    /// - `None`: No state update jobs have completed yet
    /// - `Some(block_number)`: Highest block number with completed state updates
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database access
    ///
    /// # Returns
    /// * `Result<Option<u64>>` - Latest state update block or database error
    ///
    /// # Panics
    /// - If the database returns the StateTransition job with non-StateUpdate metadata
    async fn get_latest_completed_state_update_block(&self, config: &Arc<Config>) -> Result<Option<u64>> {
        let db = config.database();

        // Get the latest completed StateTransition job
        match db.get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await? {
            Some(job_item) => match job_item.metadata.specific {
                // Match based on state update context type
                // Block - Settling without applicative recursion (i.e., L3)
                // Batch - Settling with applicative recursion (i.e., L2)
                JobSpecificMetadata::StateUpdate(metadata) => match metadata.context {
                    // Return the max block from the last state transition job
                    SettlementContext::Block(data) => Ok(data.to_settle.iter().max().copied()),
                    SettlementContext::Batch(data) => {
                        // Get the last batch from the last state transition job
                        let last_settled_batch = data.to_settle.iter().max().copied();
                        match last_settled_batch {
                            Some(last_settled_batch_num) => {
                                // Get the batch details for the last-settled batch
                                let batch = db.get_batches_by_indexes(vec![last_settled_batch_num]).await?;
                                if batch.is_empty() {
                                    Err(eyre!("Failed to fetch latest batch {} from database", last_settled_batch_num))
                                } else {
                                    // Return the end block of the last batch
                                    Ok(Some(batch[0].end_block))
                                }
                            }
                            None => Ok(None),
                        }
                    }
                },
                _ => panic!("Unexpected metadata type for StateUpdate job"),
            },
            // No completed StateTransition job, so no completed state update block
            None => Ok(None),
        }
    }

    /// Counts the number of pending SNOS jobs that consume concurrency slots.
    ///
    /// This method counts jobs in states that occupy concurrency slots:
    /// - **PendingRetry**: Jobs that failed and are waiting to be retried
    /// - **Created**: Jobs that have been created but not yet started processing
    ///
    /// These jobs reduce the available slots for new job creation since they
    /// will eventually consume processing resources.
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database access
    ///
    /// # Returns
    /// * `Result<u64>` - Count of pending jobs or database error
    async fn count_pending_snos_jobs(&self, config: &Arc<Config>, max_slots: Option<i64>) -> Result<u64> {
        let db = config.database();
        let pending_statuses = vec![JobStatus::PendingRetry, JobStatus::Created];

        let pending_jobs = db
            .get_jobs_by_types_and_statuses(vec![JobType::SnosRun], pending_statuses, max_slots)
            .await
            .wrap_err("Failed to fetch pending SNOS jobs")?;

        Ok(pending_jobs.len() as u64)
    }

    /// Retrieves missing block numbers within a specified range, respecting slot limits.
    ///
    /// This method queries the database to find blocks that need SNOS processing
    /// within the given range. It respects the slot limit to prevent over-scheduling.
    ///
    /// # Processing Logic
    /// - Queries database for blocks without SNOS jobs in the range
    /// - Limits results to available slot count
    /// - Returns blocks in database-determined order (typically ascending)
    ///
    /// # Arguments
    /// * `config` - Application configuration containing database access
    /// * `start` - Starting block number (inclusive)
    /// * `end` - Ending block number (inclusive)
    /// * `limit` - Maximum number of blocks to return (respects available slots)
    ///
    /// # Returns
    /// * `Result<Vec<u64>>` - List of missing block numbers or database error
    ///
    /// # Guarantees
    /// - Result length ≤ limit
    /// - All returned blocks are in range [start, end]
    /// - Blocks have no existing SNOS jobs in any state
    async fn get_missing_blocks_in_range(
        &self,
        config: &Arc<Config>,
        start: u64,
        end: u64,
        limit: u64,
    ) -> Result<Vec<u64>> {
        let db = config.database();
        let blocks = db
            .get_missing_block_numbers_by_type_and_caps(JobType::SnosRun, start, end, Some(i64::try_from(limit)?))
            .await?;

        Ok(blocks)
    }
}

async fn create_jobs_snos(config: Arc<Config>, block_numbers_to_pocesss: Vec<u64>) -> Result<()> {
    for block_num in block_numbers_to_pocesss {
        let starknet_version = match fetch_block_starknet_version(&config, block_num).await {
            Ok(version) => {
                if !is_starknet_version_supported(&version) {
                    error!(
                        block_id = %block_num,
                        starknet_version = %version,
                        "Block has unsupported Starknet version, skipping job creation"
                    );
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::SnosRun)),
                        KeyValue::new("operation_type", "create_job"),
                        KeyValue::new("failure_reason", "unsupported_starknet_version"),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                    continue;
                }

                info!(
                    block_id = %block_num,
                    starknet_version = %version,
                    "Validated Starknet version for block"
                );
                Some(version)
            }
            Err(e) => {
                warn!(
                    block_id = %block_num,
                    error = %e,
                    "Failed to fetch Starknet version for block, proceeding without version info"
                );
                None
            }
        };

        let metadata = create_job_metadata(block_num, config.snos_config().snos_full_output, starknet_version);

        match JobHandlerService::create_job(JobType::SnosRun, block_num.to_string(), metadata, config.clone()).await {
            Ok(_) => info!("Successfully created new Snos job: {}", block_num),
            Err(e) => {
                warn!(block_id = %block_num, error = %e, "Failed to create new Snos job");
                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::SnosRun)),
                    KeyValue::new("operation_type", format!("{:?}", "create_job")),
                ];
                ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
            }
        }
    }
    Ok(())
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
async fn fetch_block_starknet_version(config: &Arc<Config>, block_number: u64) -> Result<String> {
    use starknet::core::types::BlockId;
    use starknet::providers::Provider;

    let provider = config.madara_client();
    debug!("Fetching block header for block {} to extract Starknet version", block_number);

    // Fetch block with transaction hashes (lighter than full txs)
    let block = provider
        .get_block_with_tx_hashes(BlockId::Number(block_number))
        .await
        .wrap_err(format!("Failed to fetch block {} from sequencer", block_number))?;

    let starknet_version = match block {
        starknet::core::types::MaybePendingBlockWithTxHashes::Block(block) => block.starknet_version,
        starknet::core::types::MaybePendingBlockWithTxHashes::PendingBlock(_) => {
            return Err(eyre!("Block {} is still pending, cannot determine final Starknet version", block_number));
        }
    };

    Ok(starknet_version)
}

/// create_job_metadata is a helper function to create job metadata for a given block number and layer.
///
/// # Arguments
/// * `block_number` - The block number to process
/// * `full_output` - Set to true if layer is L3, false otherwise
/// * `starknet_version` - The Starknet protocol version for this block (from block header)
///
/// # Returns
/// * `JobMetadata` - Complete job metadata with common and SNOS-specific fields
fn create_job_metadata(block_number: u64, full_output: bool, starknet_version: Option<String>) -> JobMetadata {
    JobMetadata {
        common: CommonMetadata { starknet_version, ..Default::default() },
        specific: JobSpecificMetadata::Snos(SnosMetadata {
            block_number,
            full_output,
            cairo_pie_path: Some(format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME)),
            on_chain_data_path: Some(format!("{}/{}", block_number, ON_CHAIN_DATA_FILE_NAME)),
            snos_output_path: Some(format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)),
            ..Default::default()
        }),
    }
}
