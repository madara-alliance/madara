use crate::core::config::Config;
use crate::types::constant::BLOB_DATA_FILE_NAME;
use crate::types::jobs::metadata::{CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use opentelemetry::KeyValue;
use std::sync::Arc;

pub struct DataSubmissionJobTrigger;

#[async_trait]
impl JobTrigger for DataSubmissionJobTrigger {
    // 0. All ids are assumed to be block numbers.
    // 1. Fetch the latest completed Proving jobs without Data Submission jobs as successor jobs
    // 2. Create jobs.
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "DataSubmissionWorker", "DataSubmissionWorker started.");

        let earliest_failed_block = config.database().get_earliest_failed_block_number().await?;

        let successful_proving_jobs = config
            .database()
            .get_jobs_without_successor(JobType::ProofCreation, JobStatus::Completed, JobType::DataSubmission)
            .await?;

        for proving_job in successful_proving_jobs {
            // Extract proving metadata
            let proving_metadata: ProvingMetadata = proving_job.metadata.specific.try_into().map_err(|e| {
                tracing::error!(
                    job_id = %proving_job.internal_id,
                    error = %e,
                    "Invalid metadata type for proving job"
                );
                e
            })?;

            // Skip jobs with block numbers >= earliest failed block
            if let Some(earliest_failed_block_number) = earliest_failed_block {
                if proving_metadata.block_number >= earliest_failed_block_number {
                    tracing::debug!(
                        job_id = %proving_job.internal_id,
                        block_number = proving_metadata.block_number,
                        earliest_failed_block = earliest_failed_block_number,
                        "Skipping Data Submission job due to failed block constraint"
                    );
                    continue;
                }
            }

            // Create DA metadata
            let da_metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Da(DaMetadata {
                    block_number: proving_metadata.block_number,
                    // Set the blob data path using block number
                    blob_data_path: Some(format!("{}/{BLOB_DATA_FILE_NAME}", proving_metadata.block_number)),
                    // These will be populated during processing
                    tx_hash: None,
                }),
            };

            match JobHandlerService::create_job(
                JobType::DataSubmission,
                proving_job.internal_id.clone(),
                da_metadata,
                config.clone(),
            )
            .await
            {
                Ok(_) => tracing::info!(
                    block_id = %proving_job.internal_id,
                    "Successfully created new data submission job"
                ),
                Err(e) => {
                    tracing::warn!(
                        block_id = %proving_job.internal_id,
                        error = %e,
                        "Failed to create new data submission job"
                    );
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::DataSubmission)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        tracing::trace!(log_type = "completed", category = "DataSubmissionWorker", "DataSubmissionWorker completed.");
        Ok(())
    }
}
