use std::cmp::{max, min};
use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::WrapErr;
use opentelemetry::KeyValue;
use starknet::providers::Provider;

use crate::config::Config;
use crate::constants::{CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::jobs::create_job;
use crate::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::jobs::types::JobType;
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::workers::Worker;

pub struct SnosWorker;

#[async_trait]
impl Worker for SnosWorker {
    /// 1. Fetch the latest completed block from the Starknet chain
    /// 2. Fetch the last block that had a SNOS job run.
    /// 3. Create SNOS run jobs for all the remaining blocks
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "SnosWorker", "SnosWorker started.");

        let provider = config.starknet_client();
        let block_number_provider = provider.block_number().await?;

        let latest_block_number = if let Some(max_block_to_process) = config.service_config().max_block_to_process {
            min(max_block_to_process, block_number_provider)
        } else {
            block_number_provider
        };

        tracing::debug!(latest_block_number = %latest_block_number, "Fetched latest block number from starknet");

        let latest_job_in_db = config.database().get_latest_job_by_type(JobType::SnosRun).await?;

        let latest_job_id = latest_job_in_db
            .map(|job| {
                job.internal_id
                    .parse::<u64>()
                    .wrap_err_with(|| format!("Failed to parse job internal ID: {}", job.internal_id))
            })
            .unwrap_or(Ok(0))?;

        // To be used when testing in specific block range
        let block_start = if let Some(min_block_to_process) = config.service_config().min_block_to_process {
            max(min_block_to_process, latest_job_id)
        } else {
            latest_job_id
        };

        for block_num in block_start..latest_block_number + 1 {
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
                    snos_fact: None,
                    snos_n_steps: None,
                }),
            };

            match create_job(JobType::SnosRun, block_num.to_string(), metadata, config.clone()).await {
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
        tracing::trace!(log_type = "completed", category = "SnosWorker", "SnosWorker completed.");
        Ok(())
    }
}
