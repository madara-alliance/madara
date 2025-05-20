use std::cmp::min;
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

        // Get provider and fetch latest block number from sequencer
        let provider = config.madara_client();
        let latest_block_from_sequencer =
            provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")?;

        // Get processing boundaries from config
        let service_config = config.service_config();
        let max_block_to_process = service_config
            .max_block_to_process
            .map_or(latest_block_from_sequencer, |max_block| min(max_block, latest_block_from_sequencer));
        let min_block_to_process = service_config.min_block_to_process.map_or(0, |min_block| min_block);

        tracing::debug!(min_block = %min_block_to_process, max_block = %max_block_to_process, "Block processing range determined");

        // Get all jobs with relevant statuses
        let db = config.database();

        // TODO: This process to find last_completed_block is operation heavy, i.e for big list it will take time,
        // maybe we can ask mongodb to return in descending order
        let completed_jobs = db
            .get_jobs_by_type_and_status(JobType::SnosRun, vec![JobStatus::Completed])
            .await
            .wrap_err("Failed to fetch completed SNOS jobs")?;

        // Create set of completed block numbers for efficient lookup
        let processed_blocks: HashSet<u64> =
            completed_jobs.iter().filter_map(|job| parse_block_number(&job.internal_id)).collect();

        // Find the highest block that was already processed
        let last_completed_block =
            processed_blocks.iter().max().copied().unwrap_or(min_block_to_process.saturating_sub(1));

        tracing::debug!(last_completed_block = %last_completed_block, "Found last completed SNOS block");

        // Fetch jobs in a single query with combined statuses
        let pending_statuses = vec![JobStatus::PendingRetry, JobStatus::Created];

        // Get pending/created jobs
        let pending_jobs = db
            .get_jobs_by_type_and_status(JobType::SnosRun, pending_statuses)
            .await
            .wrap_err("Failed to fetch pending/created SNOS jobs")?;

        // Create set of pending block numbers
        let pending_blocks: HashSet<u64> =
            pending_jobs.iter().filter_map(|job| parse_block_number(&job.internal_id)).collect();

        // Check job limits
        let available_job_slots = match service_config.max_concurrent_created_snos_jobs {
            Some(max_jobs) => {
                let total_pending = pending_jobs.len() as u64;
                if total_pending >= max_jobs as u64 {
                    tracing::info!(
                        max_jobs = max_jobs,
                        current_jobs = total_pending,
                        "Maximum number of pending SNOS jobs reached. Not creating new jobs."
                    );
                    return Ok(());
                }
                max_jobs as u64 - total_pending
            }
            None => max_block_to_process - last_completed_block,
        };

        // Build block processing queue (missing blocks + new blocks)
        let mut blocks_to_process = Vec::new();

        // 1. First identify missing blocks in already "completed" range
        let missing_blocks: Vec<u64> = (min_block_to_process..=last_completed_block)
            .filter(|block| !processed_blocks.contains(block) && !pending_blocks.contains(block))
            .collect();

        // 2. Then identify new blocks to process beyond the last completed block
        let candidate_blocks: Vec<u64> = ((last_completed_block + 1)..=max_block_to_process)
            .filter(|block| !pending_blocks.contains(block))
            .collect();

        // First add missing blocks (they take priority)
        let missing_blocks_to_add = missing_blocks.into_iter().take(available_job_slots as usize).collect::<Vec<_>>();

        blocks_to_process.extend(missing_blocks_to_add.iter().copied());

        // Calculate remaining slots
        let remaining_slots = available_job_slots.saturating_sub(blocks_to_process.len() as u64);

        // Add new blocks if slots available
        if remaining_slots > 0 || service_config.max_concurrent_created_snos_jobs.is_none() {
            let new_blocks_count = if service_config.max_concurrent_created_snos_jobs.is_none() {
                candidate_blocks.len()
            } else {
                min(remaining_slots as usize, candidate_blocks.len())
            };

            blocks_to_process.extend(candidate_blocks.into_iter().take(new_blocks_count));
        }

        // Sort blocks to ensure we process in ascending order
        blocks_to_process.sort();

        // Log summary
        tracing::info!(
            total_jobs_to_create = blocks_to_process.len(),
            missing_blocks = missing_blocks_to_add.len(),
            new_blocks = blocks_to_process.len() - missing_blocks_to_add.len(),
            "About to create SNOS jobs"
        );

        // Create jobs for all identified blocks
        for block_num in blocks_to_process.clone() {
            let metadata = create_job_metadata(block_num);

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
            jobs_created = blocks_to_process.len(),
            "SnosWorker completed."
        );

        Ok(())
    }
}

// Helper function to parse block numbers from job IDs
fn parse_block_number(internal_id: &str) -> Option<u64> {
    match internal_id.parse::<u64>() {
        Ok(block_num) => Some(block_num),
        Err(e) => {
            tracing::warn!(
                internal_id = %internal_id,
                error = %e,
                "Failed to parse job internal ID as block number"
            );
            None
        }
    }
}

// Helper function to create job metadata
fn create_job_metadata(block_num: u64) -> JobMetadata {
    JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Snos(SnosMetadata {
            block_number: block_num,
            full_output: false,
            cairo_pie_path: Some(format!("{}/{}", block_num, CAIRO_PIE_FILE_NAME)),
            snos_output_path: Some(format!("{}/{}", block_num, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", block_num, PROGRAM_OUTPUT_FILE_NAME)),
            ..Default::default()
        }),
    }
}
