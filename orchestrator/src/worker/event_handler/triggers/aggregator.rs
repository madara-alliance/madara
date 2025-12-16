use crate::core::config::Config;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus};
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
use tracing::{debug, error};

pub struct AggregatorJobTrigger;

#[async_trait]
impl JobTrigger for AggregatorJobTrigger {
    /// 1. Fetch all the batches for which the status is Closed
    /// 2. Check if all the child jobs for this batch are Completed
    /// 3. Create the Aggregator job for all such Batches and update the Batch status
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Get all the closed batches
        let closed_batches =
            config.database().get_aggregator_batches_by_status(AggregatorBatchStatus::Closed, Some(10)).await?;

        debug!("Found {} closed batches", closed_batches.len());

        // Process each batch
        for batch in closed_batches {
            // Check if all the child jobs are Completed
            match self.check_child_jobs_status(&batch, &config).await {
                Ok(are_completed) => {
                    if are_completed {
                        debug!(batch_id = %batch.id, batch_index = %batch.index, "All child jobs are completed");
                    } else {
                        debug!(batch_id = %batch.id, batch_index = %batch.index, "Not all child jobs are completed");
                        continue;
                    }
                }
                Err(err) => {
                    error!(batch_id = %batch.id, batch_index = %batch.index, error = %err, "Failed to check child jobs status");
                    continue;
                }
            }

            // Construct aggregator job metadata
            let metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Aggregator(AggregatorMetadata {
                    batch_num: batch.index,
                    bucket_id: batch.bucket_id,
                    download_proof: if config.params.store_audit_artifacts {
                        Some(format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, batch.index, PROOF_FILE_NAME))
                    } else {
                        None
                    },
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
                        .update_aggregator_batch_status_by_index(
                            batch.index,
                            AggregatorBatchStatus::PendingAggregatorRun,
                        )
                        .await?;
                }
                Err(e) => {
                    error!(error = %e, "Failed to create new {:?} job for {}", JobType::Aggregator, batch.index);
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::Aggregator)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }
}

impl AggregatorJobTrigger {
    /// Check if all the child jobs for blocks from `start_block` to `end_block` are Completed
    async fn check_child_jobs_status(
        &self,
        aggregator_batch: &AggregatorBatch,
        config: &Arc<Config>,
    ) -> color_eyre::Result<bool> {
        let database = config.database();

        // Fetch sorted SNOS batches from DB
        let snos_batches = database.get_snos_batches_by_aggregator_index(aggregator_batch.index).await?;
        let (first, last) = if snos_batches.is_empty() {
            return Ok(false);
        } else {
            // unwraps are safe here
            let first = snos_batches.first().unwrap();
            let last = snos_batches.last().unwrap();

            if first.start_block != aggregator_batch.start_block || last.end_block != aggregator_batch.end_block {
                return Ok(false);
            }

            let mut start = first.start_block;
            for batch in snos_batches.iter() {
                if !batch.status.is_closed() || batch.start_block != start {
                    return Ok(false);
                } else {
                    start = batch.end_block + 1;
                }
            }

            (first.index, last.index)
        };

        let jobs =
            database.get_jobs_between_internal_ids(JobType::ProofCreation, JobStatus::Completed, first, last).await?;
        Ok(jobs.len() == (last - first + 1) as usize)
    }
}
