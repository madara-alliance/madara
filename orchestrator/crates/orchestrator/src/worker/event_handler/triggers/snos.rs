use std::sync::Arc;
use std::u64;

use crate::core::config::Config;
use crate::types::constant::{CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use color_eyre::eyre::{Result, WrapErr};
use opentelemetry::KeyValue;
use starknet::providers::Provider;
use std::cmp::{max, min};

pub struct SnosJobTrigger;

#[async_trait]
impl JobTrigger for SnosJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> Result<()> {
        // Get the minimum and the maximum bounds

        // Get provider and fetch latest block number from sequencer
        let provider = config.madara_client();
        // Will always be in range 0..u64::MAX
        let latest_created_block_from_sequencer =
            provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")?;

        // Get processing boundaries from config
        let service_config = config.service_config();
        // Will always be in range 0..u64::MAX
        let max_block_to_process_bound = service_config.max_block_to_process;
        // Will always be in range 0..u64::MAX
        let min_block_to_process_bound = service_config.min_block_to_process;

        let mut available_slots = service_config.max_concurrent_created_snos_jobs;

        // Get all jobs with relevant statuses
        let db = config.database();

        let latest_completed_snos_job_block_number =
            db.get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed).await?;
        let latest_completed_state_update_job_block_number =
            db.get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await?;

        let lower_limit_inclusive = match latest_completed_state_update_job_block_number {
            None => min_block_to_process_bound,
            Some(job_item) => max(
                min_block_to_process_bound,
                match job_item.metadata.specific {
                    JobSpecificMetadata::StateUpdate(metadata) => {
                        *metadata.blocks_to_settle.iter().max().unwrap_or(&0_u64)
                    }
                    _ => {
                        panic! {"This case should never have been executed!"}
                    }
                },
            ),
        };

        let middle_limit = match latest_completed_snos_job_block_number {
            None => max_block_to_process_bound,
            Some(job_item) => min(
                max_block_to_process_bound,
                match job_item.metadata.specific {
                    JobSpecificMetadata::Snos(metadata) => metadata.block_number,
                    _ => {
                        panic! {"This case should never have been executed!"}
                    }
                },
            ),
        };

        let upper_limit_inclusive = min(max_block_to_process_bound, latest_created_block_from_sequencer);

        // Step 1 : Decrease slots by already number of already existing PendingRetry & Created jobs

        let pending_statuses = vec![JobStatus::PendingRetry, JobStatus::Created];

        // Get pending/created jobs
        let pending_jobs_length = db
            .get_jobs_by_types_and_statuses(vec![JobType::SnosRun], pending_statuses, None)
            .await
            .wrap_err("Failed to fetch pending/created SNOS jobs")?
            .len();

        available_slots = available_slots.saturating_sub(pending_jobs_length as u64);

        if available_slots <= 0 {
            tracing::info!("All slots occupied by pre-existing jobs, skipping creation");
            return Ok(());
        }

        // Step 2 : Get the missing blocks list and consume available_slots many to create jobs for.

        // TODO: why do we need to send as i64
        let missing_block_numbers_list = db
            .get_missing_block_numbers_by_type_and_caps(
                JobType::SnosRun,
                i64::try_from(lower_limit_inclusive)?,
                i64::try_from(middle_limit)?,
            )
            .await?;

        // Creting the list of blocks to process for as a space defined vector.
        let mut block_numbers_to_pocesss: Vec<u64> =
            missing_block_numbers_list.into_iter().take(available_slots as usize).collect();

        available_slots = available_slots.saturating_sub(block_numbers_to_pocesss.len() as u64);

        if available_slots <= 0 {
            tracing::info!("All slots occupied by pre-existing jobs, skipping creation");
            return Ok(());
        }

        // Step 3 : Get the new blocks list that we can possible process.

        // All the blocks in range of available_slots after the middle limit +1 are to be processed.
        block_numbers_to_pocesss
            .extend((middle_limit + 1)..(min(middle_limit + 1 + available_slots, upper_limit_inclusive)));

        // Sort blocks to ensure we process in ascending order
        block_numbers_to_pocesss.sort();

        tracing::info!("About to create {} snos jobs", &block_numbers_to_pocesss.len());

        tracing::info!("About to create all {:?} snos jobs", &block_numbers_to_pocesss);

        // Create jobs for all identified blocks
        for block_num in block_numbers_to_pocesss.clone() {
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
            jobs_created = &block_numbers_to_pocesss.len(),
            "SnosWorker completed."
        );

        Ok(())
    }
}

// // Helper function to parse block numbers from job IDs
// fn parse_block_number(internal_id: &str) -> Option<u64> {
//     match internal_id.parse::<u64>() {
//         Ok(block_num) => Some(block_num),
//         Err(e) => {
//             tracing::warn!(
//                 internal_id = %internal_id,
//                 error = %e,
//                 "Failed to parse job internal ID as block number"
//             );
//             None
//         }
//     }
// }

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
