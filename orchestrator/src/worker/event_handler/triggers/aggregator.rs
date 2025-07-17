use crate::core::config::Config;
use crate::types::batch::BatchStatus;
use crate::types::constant::{
    CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, PROOF_FILE_NAME, SNOS_OUTPUT_FILE_NAME, STORAGE_ARTIFACTS_DIR,
    STORAGE_BLOB_DIR,
};
use crate::types::jobs::metadata::{AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use opentelemetry::KeyValue;
use std::sync::Arc;

pub struct AggregatorJobTrigger;

#[async_trait]
impl JobTrigger for AggregatorJobTrigger {
    /// 1. Fetch all the batches for which the status is Closed
    /// 2. Check if all the child jobs for this batch are Completed
    /// 3. Create the Aggregator job for all such Batches and update the Batch status
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::info!(log_type = "starting", category = "AggregatorWorker", "AggregatorWorker started.");

        // Get all the closed batches
        let closed_batches = config.database().get_batches_by_status(BatchStatus::Closed, Some(10)).await?;

        tracing::debug!("Found {} closed batches", closed_batches.len());

        // Process each batch
        for batch in closed_batches {
            // Check if all the child jobs are Completed
            match self.check_child_jobs_status(batch.start_block, batch.end_block, config.clone()).await {
                Ok(are_completed) => {
                    if are_completed {
                        tracing::debug!(batch_id = %batch.id, batch_index = %batch.index, "All child jobs are completed");
                    } else {
                        tracing::debug!(batch_id = %batch.id, batch_index = %batch.index, "Not all child jobs are completed");
                        continue;
                    }
                }
                Err(err) => {
                    tracing::error!(batch_id = %batch.id, batch_index = %batch.index, "Failed to check child jobs status : {} ", err);
                    continue;
                }
            }

            // Get the bucket_id from batch
            let bucket_id = match batch.bucket_id {
                Some(bucket_id) => bucket_id,
                None => {
                    tracing::error!(batch_id = %batch.id, batch_index = %batch.index, "Bucket ID not found for batch");
                    continue;
                }
            };

            // Construct aggregator job metadata
            let metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Aggregator(AggregatorMetadata {
                    batch_num: batch.index,
                    bucket_id,
                    num_blocks: batch.num_blocks,
                    download_proof: Some(format!(
                        "{}/batch/{}/{}",
                        STORAGE_ARTIFACTS_DIR, batch.index, PROOF_FILE_NAME
                    )),
                    blob_data_path: format!("{}/batch/{}", STORAGE_BLOB_DIR, batch.index),
                    cairo_pie_path: format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, batch.index, CAIRO_PIE_FILE_NAME),
                    snos_output_path: format!(
                        "{}/batch/{}/{}",
                        STORAGE_ARTIFACTS_DIR, batch.index, SNOS_OUTPUT_FILE_NAME
                    ),
                    program_output_path: format!(
                        "{}/batch/{}/{}",
                        STORAGE_ARTIFACTS_DIR, batch.index, PROGRAM_OUTPUT_FILE_NAME
                    ),
                    ..AggregatorMetadata::default()
                }),
            };

            // Create a new job
            match JobHandlerService::create_job(JobType::Aggregator, batch.index.to_string(), metadata, config.clone())
                .await
            {
                Ok(_) => {
                    config
                        .database()
                        .update_batch_status_by_index(batch.index, BatchStatus::PendingAggregatorRun)
                        .await?;
                    tracing::info!(batch_id = %batch.id, batch_index = %batch.index, "Successfully created new aggregator job")
                }
                Err(_) => {
                    tracing::warn!(batch_id = %batch.id, batch_index = %batch.index, "Failed to create new aggregator job");
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::Aggregator)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        tracing::trace!(log_type = "completed", category = "AggregatorWorker", "AggregatorWorker completed.");
        Ok(())
    }
}

impl AggregatorJobTrigger {
    /// Check if all the child jobs for blocks from `start_block` to `end_block` are Completed
    async fn check_child_jobs_status(
        &self,
        start_block: u64,
        end_block: u64,
        config: Arc<Config>,
    ) -> color_eyre::Result<bool> {
        let jobs = config
            .database()
            .get_jobs_between_internal_ids(JobType::ProofCreation, JobStatus::Completed, start_block, end_block)
            .await?;
        Ok(jobs.len() == (end_block - start_block + 1) as usize)
    }

    /// Convert &str to u64
    fn str_to_u64(&self, str: &str) -> color_eyre::Result<u64> {
        match str.parse::<u64>() {
            Ok(num) => Ok(num),
            Err(_) => Err(color_eyre::eyre::eyre!("Failed to  parse string as number: {}", str)),
        }
    }
}
