use crate::core::config::Config;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus};
use crate::types::constant::{
    get_batch_artifact_file, get_batch_blob_dir, CAIRO_PIE_FILE_NAME, DA_SEGMENT_FILE_NAME, ORCHESTRATOR_VERSION,
    PROGRAM_OUTPUT_FILE_NAME, PROOF_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::metadata::{AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::{calculate_jobs_to_create, JobTrigger};
use async_trait::async_trait;
use opentelemetry::KeyValue;
use std::sync::Arc;
use tracing::{debug, error, info};

pub struct AggregatorJobTrigger;

#[async_trait]
impl JobTrigger for AggregatorJobTrigger {
    /// 1. Gate creation on the aggregator buffer size: if the
    ///    `[oldest-incomplete, latest]` internal_id window is already at
    ///    `aggregator_job_buffer_size`, skip — we'd otherwise pile up Created
    ///    jobs behind a stalled lower-block aggregator and block sequential
    ///    state updates.
    /// 2. Fetch up to `max_to_create` batches with status Closed
    /// 3. Check if all the child jobs for each batch are Completed
    /// 4. Create the Aggregator job for all such batches and update the batch status
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        let buffer_size = config.service_config().aggregator_job_buffer_size;
        let max_to_create = calculate_jobs_to_create(&config, JobType::Aggregator, buffer_size).await?;

        if max_to_create == 0 {
            debug!(buffer_size = %buffer_size, "Aggregator job buffer is full, no new jobs to create. Returning safely.");
            return Ok(());
        }

        info!("Creating max {} {:?} jobs", max_to_create, JobType::Aggregator);

        // Convert the `u64` cap to the `i64` Mongo expects.
        // A misconfigured `aggregator_job_buffer_size` above `i64::MAX` would
        // silently wrap to a negative limit with `as i64`; use `try_into` so
        // it fails loudly instead.
        let batch_fetch_limit: i64 = max_to_create.try_into().map_err(|_| {
            color_eyre::eyre::eyre!(
                "aggregator_job_buffer_size resolved to {} (> i64::MAX); refusing to query with a wrapped negative limit",
                max_to_create
            )
        })?;

        // Fetch up to `batch_fetch_limit` closed batches — no point pulling
        // more than we can legitimately turn into jobs this tick.
        let closed_batches = config
            .database()
            .get_aggregator_batches_by_status(
                AggregatorBatchStatus::Closed,
                Some(batch_fetch_limit),
                Some(ORCHESTRATOR_VERSION.to_string()),
            )
            .await?;

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
                    bucket_id: Some(batch.bucket_id),
                    download_proof: if config.params.store_audit_artifacts {
                        Some(get_batch_artifact_file(batch.index, PROOF_FILE_NAME))
                    } else {
                        None
                    },
                    blob_data_path: get_batch_blob_dir(batch.index),
                    da_segment_path: get_batch_artifact_file(batch.index, DA_SEGMENT_FILE_NAME),
                    cairo_pie_path: get_batch_artifact_file(batch.index, CAIRO_PIE_FILE_NAME),
                    snos_output_path: get_batch_artifact_file(batch.index, SNOS_OUTPUT_FILE_NAME),
                    program_output_path: get_batch_artifact_file(batch.index, PROGRAM_OUTPUT_FILE_NAME),
                    ..AggregatorMetadata::default()
                }),
            };

            // Create a new job
            match JobHandlerService::create_job(JobType::Aggregator, batch.index, metadata, config.clone()).await {
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
                    MetricsRecorder::record_failed_job_operation(1.0, &attributes);
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
            let first_snos_batch = snos_batches.first().unwrap();
            let last_snos_batch = snos_batches.last().unwrap();

            // checking if the boundaries of aggregator batch snos batches are same
            if first_snos_batch.start_block != aggregator_batch.start_block
                || last_snos_batch.end_block != aggregator_batch.end_block
            {
                return Ok(false);
            }

            // confirming that there are no gaps in the batches and all snos batches are closed
            let mut expected_start_block = first_snos_batch.start_block;
            for batch in snos_batches.iter() {
                if !batch.status.is_closed() || batch.start_block != expected_start_block {
                    return Ok(false);
                } else {
                    expected_start_block = batch.end_block + 1;
                }
            }

            (first_snos_batch.index, last_snos_batch.index)
        };

        let jobs =
            database.get_jobs_between_internal_ids(JobType::ProofCreation, JobStatus::Completed, first, last).await?;
        Ok(jobs.len() == (last - first + 1) as usize)
    }
}
