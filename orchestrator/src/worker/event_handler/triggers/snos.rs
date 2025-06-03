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
use std::sync::Arc;

pub struct SnosJobTrigger;

#[async_trait]
impl JobTrigger for SnosJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> Result<()> {
        // // // Part 1: Defining Variables // // //

        // Get provider and fetch latest block number from sequencer
        let provider = config.madara_client();
        // Possible Values : 0..u64::MAX or Failed (network error or no block in Madara)
        let latest_created_block_from_sequencer =
            provider.block_number().await.wrap_err("Failed to fetch latest block number from sequencer")?;

        // Get processing boundaries from config
        let service_config = config.service_config();
        // Will always be in range 0..u64::MAX or None.
        let max_block_to_process_bound = service_config.max_block_to_process;
        // Will always be in range 0..u64::MAX
        let min_block_to_process_bound = service_config.min_block_to_process;

        // Get all jobs with relevant statuses
        let db = config.database();

        let latest_snos_completed_block_number =
            match db.get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed).await? {
                None => None,
                Some(job_item) => match job_item.metadata.specific {
                    JobSpecificMetadata::Snos(metadata) => Some(metadata.block_number),
                    _ => {
                        panic! {"This case should never have happened!"}
                    }
                },
            };

        let latest_su_completed_block_number =
            match db.get_latest_job_by_type_and_status(JobType::StateTransition, JobStatus::Completed).await? {
                None => None,
                Some(job_item) => match job_item.metadata.specific {
                    JobSpecificMetadata::StateUpdate(metadata) => metadata.blocks_to_settle.iter().max().copied(),
                    _ => {
                        panic! {"This case should never have happened!"}
                    }
                },
            };

        let lower_limit = match latest_su_completed_block_number {
            None => min_block_to_process_bound,
            Some(value) => max(value, min_block_to_process_bound),
        };

        let middle_limit = latest_snos_completed_block_number;

        let upper_limit = match max_block_to_process_bound {
            Some(bound) => min(latest_created_block_from_sequencer, bound),
            // No limit, use sequencer value
            None => latest_created_block_from_sequencer,
        };

        let mut block_numbers_to_pocesss: Vec<u64> = Vec::new();

        // // // Part 2: Calculating Available Slots // // //

        let mut available_slots = service_config.max_concurrent_created_snos_jobs;

        // Decrease slots by number of already existing PendingRetry & Created jobs
        let pending_statuses = vec![JobStatus::PendingRetry, JobStatus::Created];

        // Get pending/created jobs
        let pending_jobs_length = db
            .get_jobs_by_types_and_statuses(vec![JobType::SnosRun], pending_statuses, None)
            .await
            .wrap_err("Failed to fetch pending SNOS jobs")?
            .len();

        available_slots = available_slots.saturating_sub(pending_jobs_length as u64);

        if available_slots == 0 {
            tracing::warn!("All slots occupied by pre-existing jobs, skipping SNOS job creation!");
            return Ok(());
        }

        // // // Part 3: Calculating jobs to process // // //

        // NOTE:
        // The only variables we are to be concerned with from now are
        // lower_limit, middle_limit, upper_limit and available_slots

        // Case: middle_limit is NONE, i.e no snos job has completed till now.
        // lower limit can be [0...u64::MAX]
        if middle_limit.is_none() {
            // create jobs for all blocks in range [lower_limit, upper_limit] - blocks already having a Snos Job
            let candidate_blocks = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    lower_limit,
                    upper_limit,
                    Some(i64::try_from(available_slots)?),
                )
                .await?;

            assert!(candidate_blocks.len() <= available_slots as usize);

            // create_jobs_snos
            tracing::info!(
                "Creating SNOS jobs for {:?} blocks, with {} left slots",
                &candidate_blocks,
                available_slots
            );
            create_jobs_snos(config, candidate_blocks).await?;
            return Ok(());
        }

        let middle_limit = middle_limit.expect("middle_limit should not be None at this point");

        // // // Part 4a: Calculating jobs withing first half // // //

        // Case 1: lower_limit > middle_limit : Skip, do nothing
        // Case 2: lower_limit == middle_limit == 0 : Skip, already processed
        // Case 3: lower_limit <= middle_limit && (middle_limit != 0) : Check missing_blocks

        if lower_limit <= middle_limit && (middle_limit != 0) {
            // Get the missing blocks list and consume available_slots many to create jobs for.
            let missing_block_numbers_list = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    lower_limit,
                    middle_limit,
                    Some(i64::try_from(available_slots)?),
                )
                .await?;

            assert!(missing_block_numbers_list.len() <= available_slots as usize);

            available_slots = available_slots.saturating_sub(missing_block_numbers_list.len() as u64);
            block_numbers_to_pocesss.extend(missing_block_numbers_list);
        };

        if available_slots == 0 {
            // Create the jobs here directly and return!
            tracing::info!(
                "All available slots are now full, creating jobs for blocks {:?}",
                &block_numbers_to_pocesss,
            );
            create_jobs_snos(config, block_numbers_to_pocesss).await?;
            return Ok(());
        }

        // // // Part 4b: Calculating jobs withing second half // // //

        // Case 1: middle_limit > upper_limit : Skip, do nothing
        // Case 2: middle_limit = upper_limit : Skip, nothing to do
        // Case 3: middle_limit < upper_limit : Check for candidate_blocks

        if middle_limit < upper_limit {
            // Get the candidate blocks list and consume available_slots many to create jobs for.
            let candidate_blocks = db
                .get_missing_block_numbers_by_type_and_caps(
                    JobType::SnosRun,
                    middle_limit,
                    upper_limit,
                    Some(i64::try_from(available_slots)?),
                )
                .await?;

            assert!(candidate_blocks.len() <= available_slots as usize);

            available_slots = available_slots.saturating_sub(candidate_blocks.len() as u64);
            block_numbers_to_pocesss.extend(candidate_blocks);
        };

        // // // Part 5: Creating the jobs // // //

        tracing::info!(
            "Creating SNOS jobs for {:?} blocks, with {} left slots",
            &block_numbers_to_pocesss,
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
            Ok(_) => tracing::info!("Successfully created new Snos job: {}", block_num),
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
