use crate::config::config;
use crate::jobs::create_job;
use crate::jobs::types::JobType;
use crate::workers::Worker;
use async_trait::async_trait;
use starknet::providers::Provider;
use std::collections::HashMap;
use std::error::Error;

pub struct SnosWorker;

#[async_trait]
impl Worker for SnosWorker {
    /// 1. Fetch the latest completed block from the Starknet chain
    /// 2. Fetch the last block that had a SNOS job run.
    /// 3. Create SNOS run jobs for all the remaining blocks
    async fn run_worker(&self) -> Result<(), Box<dyn Error>> {
        let config = config().await;
        let provider = config.starknet_client();
        let latest_block_number = provider.block_number().await?;
        let latest_block_processed_data = config
            .database()
            .get_latest_job_by_type_and_internal_id(JobType::SnosRun)
            .await
            .unwrap()
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());

        let latest_block_processed: u64 = latest_block_processed_data.parse()?;

        let block_diff = latest_block_number - latest_block_processed;

        // if all blocks are processed
        if block_diff == 0 {
            return Ok(());
        }

        for x in latest_block_processed + 1..latest_block_number + 1 {
            create_job(JobType::SnosRun, x.to_string(), HashMap::new()).await?;
        }

        Ok(())
    }
}
