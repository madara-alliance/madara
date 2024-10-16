use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use starknet::providers::Provider;

use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::workers::Worker;

pub struct SnosWorker;

#[async_trait]
impl Worker for SnosWorker {
    /// 1. Fetch the latest completed block from the Starknet chain
    /// 2. Fetch the last block that had a SNOS job run.
    /// 3. Create SNOS run jobs for all the remaining blocks
    async fn run_worker(&self, config: Arc<Config>) -> Result<(), Box<dyn Error>> {
        tracing::trace!(log_type = "starting", category = "SnosWorker", "SnosWorker started.");

        let provider = config.starknet_client();
        let latest_block_number = provider.block_number().await?;
        tracing::debug!(latest_block_number = %latest_block_number, "Fetched latest block number from Starknet");

        let latest_block_processed_data = config
            .database()
            .get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed)
            .await?
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());
        tracing::debug!(latest_processed_block = %latest_block_processed_data, "Fetched latest processed block from database");

        // Check if job does not exist
        // TODO: fetching all SNOS jobs with internal id > latest_block_processed_data
        // can be done in one DB call
        let job_in_db = config
            .database()
            .get_job_by_internal_id_and_type(&latest_block_number.to_string(), &JobType::SnosRun)
            .await?;

        if job_in_db.is_some() {
            tracing::trace!(block_number = %latest_block_number, "SNOS job already exists for the latest block");
            return Ok(());
        }

        let latest_block_processed: u64 = match latest_block_processed_data.parse() {
            Ok(block) => block,
            Err(e) => {
                tracing::error!(error = %e, block_no = %latest_block_processed_data, "Failed to parse latest processed block number");
                return Err(Box::new(e));
            }
        };

        let block_diff = latest_block_number - latest_block_processed;
        tracing::debug!(block_diff = %block_diff, "Calculated block difference");

        // if all blocks are processed
        if block_diff == 0 {
            tracing::info!("All blocks are already processed");
            return Ok(());
        }

        for x in latest_block_processed + 1..latest_block_number + 1 {
            create_job(JobType::SnosRun, x.to_string(), HashMap::new(), config.clone()).await?;
        }
        tracing::trace!(log_type = "completed", category = "SnosWorker", "SnosWorker completed.");
        Ok(())
    }
}
