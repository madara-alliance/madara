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
        println!("HEEMANK #1: Starting Snos Worker");

        // // // Part 1: Defining Variables // // //

        // Get provider and fetch latest block number from sequencer
        let provider = config.madara_client();
        // Possible Values : 0..u64::MAX or Failed (network error or no block in Madara)
        let latest_created_block_from_sequencer =
            provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")?;

        println!("HEEMANK #2: latest_created_block_from_sequencer is {}", &latest_created_block_from_sequencer);

        // Get processing boundaries from config
        let service_config = config.service_config();
        // Will always be in range 0..u64::MAX
        let max_block_to_process_bound = service_config.max_block_to_process;
        // Will always be in range 0..u64::MAX
        let min_block_to_process_bound = service_config.min_block_to_process;

        println!("HEEMANK #3 max_block_to_process_bound is {}", &max_block_to_process_bound);
        println!("HEEMANK #4 min_block_to_process_bound is {}", &min_block_to_process_bound);

        // Get all jobs with relevant statuses
        let db = config.database();

        let latest_snos_completed_block_number =
            match db.get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed).await? {
                None => None,
                Some(job_item) => match job_item.metadata.specific {
                    JobSpecificMetadata::Snos(metadata) => Some(metadata.block_number),
                    _ => {
                        panic! {"This case should never have been executed!"}
                    }
                },
            };

        let latest_su_completed_block_number =
            match db.get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await? {
                None => None,
                Some(job_item) => match job_item.metadata.specific {
                    JobSpecificMetadata::StateUpdate(metadata) => {
                        Some(*metadata.blocks_to_settle.iter().max().unwrap_or(&0_u64))
                    }
                    _ => {
                        panic! {"This case should never have been executed!"}
                    }
                },
            };

        println!("HEEMANK #5 latest_snos_completed_block_number {:?}", latest_snos_completed_block_number);
        println!("HEEMANK #6 latest_su_completed_block_number {:?}", latest_su_completed_block_number);

        let lower_limit = match latest_su_completed_block_number {
            None => min_block_to_process_bound,
            Some(value) => max(value, min_block_to_process_bound),
        };

        let middle_limit_optn = latest_snos_completed_block_number;

        let upper_limit = min(latest_created_block_from_sequencer, max_block_to_process_bound);

        println!("HEEMANK #7 lower_limit {:?}", lower_limit);
        println!("HEEMANK #8 middle_limit {:?}", middle_limit_optn);
        println!("HEEMANK #9 upper_limit {:?}", upper_limit);

        // // // Part 2: Calculating Available Slots // // //

        let mut available_slots = service_config.max_concurrent_created_snos_jobs;

        // Decrease slots by already number of already existing PendingRetry & Created jobs
        let pending_statuses = vec![JobStatus::PendingRetry, JobStatus::Created];

        // Get pending/created jobs
        let pending_jobs_length = db
            .get_jobs_by_types_and_statuses(vec![JobType::SnosRun], pending_statuses, None)
            .await
            .wrap_err("Failed to fetch pending / created SNOS jobs")?
            .len();

        println!("HEEMANK #10 available_slots {:?}", available_slots);
        println!("HEEMANK #11 pending_jobs_length {:?}", pending_jobs_length);

        available_slots = available_slots.saturating_sub(pending_jobs_length as u64);

        println!("HEEMANK #12 available_slots after removing pending_jobs {:?}", available_slots);

        if available_slots <= 0 {
            tracing::warn!("All slots occupied by pre-existing jobs, skipping SNOS job creation!");
            return Ok(());
        }

        // // // Part 3: Calculating jobs to process // // //

        // NOTE:
        // The only variables we are to be concerned with from now are
        // lower_limit, middle_limit_optn, upper_limit and available_slots

        // Case: middle_limit_optn is NONE
        // lower limit can be 0, any, u64::MAX
        if middle_limit_optn.is_none() {
            // create jobs for all blocks in range [lower_limit, upper_limit] - blocks already having a Snos Job
            let candidate_blocks = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    i64::try_from(lower_limit)?,
                    i64::try_from(upper_limit)?,
                )
                .await?;
            // blocks.take(available_slots) into block_numbers_to_pocesss
            // create_jobs_snos
            let blocks_taken: Vec<u64> = candidate_blocks.into_iter().take(available_slots as usize).collect();
            available_slots = available_slots.saturating_sub(blocks_taken.len() as u64);
            tracing::info!(
                "Creating SNOS jobs for {} blocks, with {} left slots",
                &blocks_taken.len(),
                available_slots
            );
            create_jobs_snos(config, blocks_taken).await?;
            return Ok(());
        }

        let last_completed_snos_block_no =
            middle_limit_optn.expect("middle_limit_optn should not be None at this point");
        let mut block_numbers_to_pocesss: Vec<u64> = Vec::new();

        // // // Part 4: Calculating jobs withing first half // // //

        // Case 1: lower_limit > last_completed_snos_block_no : Skip, do nothing
        // Case 2: lower_limit == last_completed_snos_block_no == 0 : Skip, already processed
        // Case 3: lower_limit <= last_completed_snos_block_no && (last_completed_snos_block_no != 0) : Check missing_blocks

        if lower_limit <= last_completed_snos_block_no && (last_completed_snos_block_no != 0) {
            // Get the missing blocks list and consume available_slots many to create jobs for.
            // TODO: why do we need to send as i64
            let missing_block_numbers_list = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    i64::try_from(lower_limit)?,
                    i64::try_from(last_completed_snos_block_no)?,
                )
                .await?;

            let blocks_taken: Vec<u64> =
                missing_block_numbers_list.into_iter().take(available_slots as usize).collect();
            block_numbers_to_pocesss.extend(blocks_taken);
            available_slots = available_slots.saturating_sub(block_numbers_to_pocesss.len() as u64);
        };

        if available_slots <= 0 {
            // Create the jobs here directly and return!
            tracing::info!("All available slots are now full, creating jobs for blocks");
            create_jobs_snos(config, block_numbers_to_pocesss).await?;
            return Ok(());
        }

        // // // Part 5: Calculating jobs withing second half // // //

        // Case 1: last_completed_snos_block_no > upper_limit : Skip, do nothing
        // Case 2: last_completed_snos_block_no = upper_limit : Skip, nothing to do
        // Case 3: last_completed_snos_block_no < upper_limit : Check for candidate_blocks

        if last_completed_snos_block_no < upper_limit {
            // Get the candidate blocks list and consume available_slots many to create jobs for.
            // TODO: why do we need to send as i64
            let candidate_blocks = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    i64::try_from(last_completed_snos_block_no)?,
                    i64::try_from(upper_limit)?,
                )
                .await?;

            let blocks_taken: Vec<u64> = candidate_blocks.into_iter().take(available_slots as usize).collect();
            block_numbers_to_pocesss.extend(blocks_taken);
            available_slots = available_slots.saturating_sub(block_numbers_to_pocesss.len() as u64);
        };

        // Create the jobs here directly and return!
        tracing::info!(
            "Creating SNOS jobs for {} blocks, with {} left slots",
            &block_numbers_to_pocesss.len(),
            available_slots
        );
        create_jobs_snos(config, block_numbers_to_pocesss).await?;
        Ok(())
    }
}

async fn create_jobs_snos(config: Arc<Config>, block_numbers_to_pocesss: Vec<u64>) -> Result<()> {
    // Create jobs for all identified blocks
    for block_num in block_numbers_to_pocesss {
        let metadata = create_job_metadata(block_num);

        match JobHandlerService::create_job(JobType::SnosRun, block_num.to_string(), metadata, config.clone()).await {
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
    Ok(())
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
