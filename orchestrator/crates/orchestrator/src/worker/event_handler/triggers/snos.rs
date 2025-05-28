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

        println!("HEEMANK #1");

        // Get provider and fetch latest block number from sequencer
        // let provider = config.madara_client();
        // Will always be in range 0..u64::MAX
        let latest_created_block_from_sequencer =
            // provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")?;
            100000;

        println!("HEEMANK #2 {}",latest_created_block_from_sequencer);

        // Get processing boundaries from config
        let service_config = config.service_config();
        // Will always be in range 0..u64::MAX
        let max_block_to_process_bound = service_config.max_block_to_process;
        // Will always be in range 0..u64::MAX
        let min_block_to_process_bound = service_config.min_block_to_process;

        let mut available_slots = service_config.max_concurrent_created_snos_jobs;

        println!("HEEMANK #3 {} {} {}", max_block_to_process_bound, min_block_to_process_bound, available_slots);

        // Get all jobs with relevant statuses
        let db = config.database();

        let latest_completed_snos_job_block_number =
            db.get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed).await?;
        let latest_completed_state_update_job_block_number =
            db.get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await?;

        println!("HEEMANK #3.1 {:?}", latest_completed_snos_job_block_number);
        println!("HEEMANK #3.2 {:?}", latest_completed_state_update_job_block_number);


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
            None => min_block_to_process_bound,
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

        println!("HEEMANK #4 {} {} {}", lower_limit_inclusive, middle_limit, upper_limit_inclusive);

        // Step 1 : Decrease slots by already number of already existing PendingRetry & Created jobs

        let pending_statuses = vec![JobStatus::PendingRetry, JobStatus::Created];

        // Get pending/created jobs
        let pending_jobs_length = db
            .get_jobs_by_types_and_statuses(vec![JobType::SnosRun], pending_statuses, None)
            .await
            .wrap_err("Failed to fetch pending/created SNOS jobs")?
            .len();

        println!("HEEMANK #4.5 {} {}", available_slots,  pending_jobs_length);

        available_slots = available_slots.saturating_sub(pending_jobs_length as u64);


        println!("HEEMANK #5 {}", available_slots);

        if available_slots <= 0 {
            println!("All slots occupied by pre-existing jobs, skipping creation");
            return Ok(());
        }

        // Step 2 : Get the missing blocks list and consume available_slots many to create jobs for.

        println!("HEEMANK #5.1 {}", &available_slots);
        // TODO: why do we need to send as i64
        let missing_block_numbers_list = db
            .get_missing_block_numbers_by_type_and_caps(
                JobType::SnosRun,
                i64::try_from(lower_limit_inclusive)?,
                i64::try_from(middle_limit)?,
            )
            .await?;

        println!("HEEMANK #5.5 {}", &missing_block_numbers_list.len());

        // Creting the list of blocks to process for as a space defined vector.
        let mut block_numbers_to_pocesss: Vec<u64> =
            missing_block_numbers_list.into_iter().take(available_slots as usize).collect();

        available_slots = available_slots.saturating_sub(block_numbers_to_pocesss.len() as u64);

        println!("HEEMANK #5.7 {}", &available_slots);

        if available_slots > 0 {
            // Step 3 : Get the new blocks list that we can possibly process.

            let mut start_block = middle_limit + 1;
            let end_block: u64 = upper_limit_inclusive;

            // Special Case when no previous blocks exists;
            // So we make middle_limit inclusive.
            if middle_limit == 0 {
                start_block = middle_limit;
            }

            // list of all the blocks between start_blokc and upper_limit_inclusive
            // that are not having any snos jobs.
            // take into the block_numbers_to_pocesss from that.

            println!("HEEMANK #5.8 {} {}", start_block, end_block);

            let missing_block_numbers_after_middle = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    i64::try_from(start_block)?,
                    i64::try_from(end_block)?,
                )
                .await?;
            println!("HEEMANK #5.9 {:?}", &missing_block_numbers_after_middle);


            if missing_block_numbers_after_middle.len() > 0 {
                // we got blocks
                // Creting the list of blocks to process for as a space defined vector.

                block_numbers_to_pocesss =
                    missing_block_numbers_after_middle.into_iter().take(available_slots as usize).collect();

            } else {
                // there are no such blocks
                // either completely empty
                // so we need to create the jobs!
                println!("HEEMANK #5.91 {:?} {}", &(start_block + available_slots), upper_limit_inclusive );

                block_numbers_to_pocesss
                    .extend(start_block..(min(start_block + available_slots, upper_limit_inclusive)));
                // or completely done
                // so we need not do anything
                // can't happen since then last_completed_snos == max_limit.
            }

            println!("HEEMANK #5.10 {:?}", block_numbers_to_pocesss);


        }

        println!("HEEMANK #6 {}", available_slots);


        // Sort blocks to ensure we process in ascending order
        block_numbers_to_pocesss.sort();

        println!("HEEMANK #7");

        println!("About to create {} snos jobs", &block_numbers_to_pocesss.len());
        println!("About to create all {:?} snos jobs", &block_numbers_to_pocesss);

        create_jobs_snos(config, block_numbers_to_pocesss).await?;

        tracing::trace!(
            log_type = "completed",
            category = "SnosWorker",
            "SnosWorker completed."
        );

        Ok(())
    }
}

async fn create_jobs_snos(config: Arc<Config> , block_numbers_to_pocesss: Vec<u64>) -> Result<()> {
    // Create jobs for all identified blocks
    for block_num in block_numbers_to_pocesss {
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
