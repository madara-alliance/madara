use std::cmp::{max, min};
use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::{Result, WrapErr};
use opentelemetry::KeyValue;
use starknet::providers::Provider;

use crate::core::config::Config;
use crate::types::constant::{CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;

pub struct SnosJobTrigger;

#[async_trait]
impl JobTrigger for SnosJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> Result<()> {
        tracing::trace!(log_type = "starting", category = "SnosWorker", "SnosWorker started.");

        let provider = config.madara_client();

        // Get the latest block from sequencer
        let latest_block_from_sequencer =
            provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")?;

        // Get max block number to process up to
        let max_block_to_process = match config.service_config().max_block_to_process {
            Some(max_block) => min(max_block, latest_block_from_sequencer),
            None => latest_block_from_sequencer,
        };

        tracing::debug!(max_block_to_process = %max_block_to_process, "Fetched latest block number from starknet");

        // Get minimum block to process from config (default to 0 if not specified)
        let min_block_to_process = match config.service_config().min_block_to_process {
            Some(min_block) => min_block,
            None => {
                tracing::debug!("No minimum block specified in config, defaulting to 0");
                0
            }
        };

        // Fetch all completed SNOS jobs in a single database call
        let all_completed_jobs = config
            .database()
            .get_jobs_by_type_and_status(JobType::SnosRun, JobStatus::Completed)
            .await
            .wrap_err("Failed to fetch completed SNOS jobs")?;

        // Find the last completed block
        let last_completed_block = all_completed_jobs
            .iter()
            .filter_map(|job| match job.internal_id.parse::<u64>() {
                Ok(block_num) => Some(block_num),
                Err(e) => {
                    tracing::warn!(
                        job_id = %job.id,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Failed to parse job internal ID"
                    );
                    None
                }
            })
            .max();

        let last_completed_block = match last_completed_block {
            Some(block) => block,
            None => {
                tracing::info!("No previously completed SNOS jobs found, starting from min_block_to_process");
                // We use min_block_to_process - 1 so that the first block to be processed will be min_block_to_process
                // This ensures we don't incorrectly assume block 0 is processed when no blocks are processed
                min_block_to_process.saturating_sub(1)
            }
        };

        tracing::debug!(last_completed_block = %last_completed_block, "Found last completed SNOS block");

        // Create a HashSet of all processed blocks for efficient lookup
        let processed_blocks: HashSet<u64> = all_completed_jobs
            .iter()
            .filter_map(|job| match job.internal_id.parse::<u64>() {
                Ok(block_num) => Some(block_num),
                Err(e) => {
                    tracing::warn!(
                        job_id = %job.id,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Failed to parse job internal ID while building processed blocks set"
                    );
                    None
                }
            })
            .collect();

        // Fetch pending and created jobs
        let pending_jobs = config
            .database()
            .get_jobs_by_type_and_status(JobType::SnosRun, JobStatus::PendingRetry)
            .await
            .wrap_err("Failed to fetch pending SNOS jobs")?;

        let created_jobs = config
            .database()
            .get_jobs_by_type_and_status(JobType::SnosRun, JobStatus::Created)
            .await
            .wrap_err("Failed to fetch created SNOS jobs")?;

        // Create a set of blocks that are already in pending or created status
        // to avoid creating duplicate jobs for these blocks
        let pending_and_created_blocks: HashSet<u64> = pending_jobs
            .iter()
            .chain(created_jobs.iter())
            .filter_map(|job| match job.internal_id.parse::<u64>() {
                Ok(block_num) => Some(block_num),
                Err(e) => {
                    tracing::warn!(
                        job_id = %job.id,
                        internal_id = %job.internal_id,
                        error = %e,
                        "Failed to parse job internal ID while building pending/created blocks set"
                    );
                    None
                }
            })
            .collect();

        // Calculate total pending and created jobs
        let total_pending_and_created: u64 = (pending_jobs.len() + created_jobs.len()) as u64;

        // Check if we have a job limit from config
        let available_job_slots = match config.service_config().max_concurrent_created_snos_jobs {
            // If limit is set, check if we have room for more jobs
            Some(max_jobs) => {
                if total_pending_and_created >= max_jobs as u64 {
                    tracing::info!(
                        max_jobs = max_jobs,
                        current_jobs = total_pending_and_created,
                        "Maximum number of pending SNOS jobs reached. Not creating new jobs."
                    );
                    return Ok(());
                }
                max_jobs as u64 - total_pending_and_created
            }
            // If no limit is set, we can create jobs up to the max block
            None => {
                tracing::debug!("No maximum concurrent SNOS jobs limit specified in config");
                max_block_to_process - last_completed_block
            }
        };

        // Get all jobs we need to create, starting with missing blocks
        let mut jobs_to_create = Vec::new();

        // 1. First identify missing blocks in already "completed" range
        // Ensure we don't try to process blocks below min_block_to_process
        let range_start = if min_block_to_process > 0 { min_block_to_process } else { 0 };
        let missing_blocks: Vec<u64> = (range_start..=last_completed_block)
            .filter(|block| !processed_blocks.contains(block))
            // FIX: Also filter out blocks that are already pending or created
            .filter(|block| !pending_and_created_blocks.contains(block))
            .collect();

        tracing::info!(
            count = missing_blocks.len(),
            min_block = %min_block_to_process,
            last_completed_block = %last_completed_block,
            "Found missing blocks before last completed block"
        );

        // Add missing blocks to jobs queue first (they take priority)
        let missing_blocks_to_add =
            missing_blocks.iter().take(available_job_slots as usize).copied().collect::<Vec<_>>();

        jobs_to_create.extend(missing_blocks_to_add.iter());

        // Calculate remaining available slots
        let remaining_slots = if missing_blocks_to_add.len() as u64 <= available_job_slots {
            available_job_slots - missing_blocks_to_add.len() as u64
        } else {
            0
        };

        // 2. If we still have slots or no limit (None), add new blocks after the last completed one
        if remaining_slots > 0 || config.service_config().max_concurrent_created_snos_jobs.is_none() {
            //  we need to handle the case where some blocks in our desired
            // range are already pending/created by continuing to add more blocks beyond the initial range

            // Start from the block after the last completed one
            let block_start = max(min_block_to_process, last_completed_block + 1);

            // Collect all candidate blocks from block_start up to max_block_to_process
            // that aren't already in pending or created status
            let candidate_blocks: Vec<u64> = (block_start..=max_block_to_process)
                .filter(|block| !pending_and_created_blocks.contains(block))
                .collect();

            // Take only as many blocks as we have slots for
            let new_blocks_count = if config.service_config().max_concurrent_created_snos_jobs.is_none() {
                candidate_blocks.len() // No limit, take all candidate blocks
            } else {
                min(remaining_slots as usize, candidate_blocks.len())
            };

            // Take the first new_blocks_count blocks from candidate_blocks
            let new_blocks = candidate_blocks.into_iter().take(new_blocks_count).collect::<Vec<_>>();

            tracing::info!(
                new_blocks_count = %new_blocks_count,
                remaining_slots = %remaining_slots,
                "new blocks to add {:?}", new_blocks
            );

            if !new_blocks.is_empty() {
                let min_block = new_blocks.first().unwrap();
                let max_block = new_blocks.last().unwrap();
                tracing::info!(
                    block_start = %min_block,
                    end_block = %max_block,
                    count = new_blocks.len(),
                    "Creating SNOS jobs for new blocks"
                );
                jobs_to_create.extend(new_blocks);
            }
        }

        // Sort blocks to ensure we process in ascending order
        jobs_to_create.sort();

        // Log summary before creating jobs
        tracing::info!(
            total_jobs_to_create = jobs_to_create.len(),
            missing_blocks = missing_blocks_to_add.len(),
            new_blocks = jobs_to_create.len() - missing_blocks_to_add.len(),
            "About to create SNOS jobs"
        );

        let len_of_jobs = jobs_to_create.len();

        // Create jobs for all identified blocks
        for block_num in jobs_to_create {
            // Create typed metadata structure with predefined paths
            let metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Snos(SnosMetadata {
                    block_number: block_num,
                    full_output: false,
                    // Set the storage paths using block number
                    cairo_pie_path: Some(format!("{}/{}", block_num, CAIRO_PIE_FILE_NAME)),
                    snos_output_path: Some(format!("{}/{}", block_num, SNOS_OUTPUT_FILE_NAME)),
                    program_output_path: Some(format!("{}/{}", block_num, PROGRAM_OUTPUT_FILE_NAME)),
                    ..Default::default() // Ensure all other fields are set to default
                }),
            };

            match JobHandlerService::create_job(JobType::SnosRun, block_num.to_string(), metadata, config.clone()).await
            {
                Ok(_) => tracing::info!(block_id = %block_num, "Successfully created new Snos job"),
                Err(e) => {
                    tracing::warn!(block_id = %block_num, error = %e, "Failed to create new Snos job");
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::SnosRun)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        tracing::trace!(
            log_type = "completed",
            category = "SnosWorker",
            jobs_created = len_of_jobs,
            "SnosWorker completed."
        );

        Ok(())
    }
}
