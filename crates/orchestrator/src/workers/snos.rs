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
        let provider = config.starknet_client();
        let latest_block_number = provider.block_number().await?;

        let latest_block_processed_data = config
            .database()
            .get_latest_job_by_type_and_status(JobType::SnosRun, JobStatus::Completed)
            .await
            .unwrap()
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());

        // Check if job does not exist
        // TODO: fetching all SNOS jobs with internal id > latest_block_processed_data
        // can be done in one DB call
        let job_in_db = config
            .database()
            .get_job_by_internal_id_and_type(&latest_block_number.to_string(), &JobType::SnosRun)
            .await
            .unwrap();

        if job_in_db.is_some() {
            return Ok(());
        }

        let latest_block_processed: u64 = latest_block_processed_data.parse()?;

        let block_diff = latest_block_number - latest_block_processed;

        // if all blocks are processed
        if block_diff == 0 {
            return Ok(());
        }

        for x in latest_block_processed + 1..latest_block_number + 1 {
            create_job(JobType::SnosRun, x.to_string(), HashMap::new(), config.clone()).await?;
        }

        Ok(())
    }
}
