use crate::core::client::database::DatabaseError;
use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::constant::BLOB_DATA_FILE_NAME;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::{JobTrigger, ProcessingResult};
use async_trait::async_trait;
use color_eyre::eyre::Context;
use opentelemetry::KeyValue;
use std::sync::Arc;

pub struct DataSubmissionJobTrigger;

#[async_trait]
impl JobTrigger for DataSubmissionJobTrigger {
    /// Creates data submission jobs for all successful proving jobs that don't have one yet.
    ///
    /// This worker:
    /// 1. Fetches all completed proving jobs without data submission jobs
    /// 2. Validates each job can be processed (not beyond failed blocks)
    /// 3. Creates corresponding data submission jobs with proper metadata
    ///
    /// Note: All job IDs are assumed to be block numbers.
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "DataSubmissionWorker", "DataSubmissionWorker started.");

        let processing_context = ProcessingContext::new(config.clone()).await?;
        let eligible_proving_jobs = processing_context.get_eligible_proving_jobs().await?;

        tracing::debug!("Found {} successful proving jobs without data submission jobs", eligible_proving_jobs.len());

        let mut created_jobs = 0;
        let mut skipped_jobs = 0;

        for proving_job in eligible_proving_jobs {
            match self.process_proving_job(&proving_job, &processing_context).await {
                ProcessingResult::Created => created_jobs += 1,
                ProcessingResult::Skipped => skipped_jobs += 1,
                ProcessingResult::Failed => {
                    // Error already logged in process_proving_job
                    continue;
                }
            }
        }

        tracing::info!(created = created_jobs, skipped = skipped_jobs, "DataSubmissionWorker completed job processing");

        tracing::trace!(log_type = "completed", category = "DataSubmissionWorker", "DataSubmissionWorker completed.");
        Ok(())
    }
}

impl DataSubmissionJobTrigger {
    /// Processes a single proving job to create its corresponding data submission job
    async fn process_proving_job(&self, proving_job: &JobItem, context: &ProcessingContext) -> ProcessingResult {
        // Extract and validate proving metadata
        let proving_metadata = match self.extract_proving_metadata(proving_job) {
            Ok(metadata) => metadata,
            Err(_) => return ProcessingResult::Failed,
        };

        // Check if job should be skipped due to failed block constraints
        if context.should_skip_block(proving_metadata.block_number) {
            tracing::debug!(
                job_id = %proving_job.internal_id,
                block_number = proving_metadata.block_number,
                earliest_failed_block = context.earliest_failed_block,
                "Skipping data submission job due to failed block constraint"
            );
            return ProcessingResult::Skipped;
        }

        // Create and submit data submission job
        match self.create_data_submission_job(proving_job, &proving_metadata, context.config.clone()).await {
            Ok(_) => {
                tracing::info!(
                    block_number = proving_metadata.block_number,
                    job_id = %proving_job.internal_id,
                    "Successfully created data submission job"
                );
                ProcessingResult::Created
            }
            Err(_) => ProcessingResult::Failed,
        }
    }

    /// Extracts and validates proving metadata from the job
    fn extract_proving_metadata(&self, proving_job: &JobItem) -> color_eyre::Result<ProvingMetadata> {
        proving_job
            .metadata
            .specific
            .clone()
            .try_into()
            .map_err(|e| {
                tracing::error!(
                    job_id = %proving_job.internal_id,
                    error = %e,
                    "Invalid metadata type for proving job"
                );
                e
            })
            .context("Unalbe to Extract Proving Metadata")
    }

    /// Creates a data submission job with the appropriate metadata
    async fn create_data_submission_job(
        &self,
        proving_job: &JobItem,
        proving_metadata: &ProvingMetadata,
        config: Arc<Config>,
    ) -> Result<(), JobError> {
        let da_metadata = self.build_da_metadata(proving_metadata);

        tracing::debug!(
            job_id = %proving_job.internal_id,
            block_number = proving_metadata.block_number,
            "Creating data submission job for proving job"
        );

        JobHandlerService::create_job(JobType::DataSubmission, proving_job.internal_id.clone(), da_metadata, config)
            .await
            .map_err(|e| {
                tracing::warn!(
                    job_id = %proving_job.internal_id,
                    error = %e,
                    "Failed to create data submission job"
                );

                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::DataSubmission)),
                    KeyValue::new("operation_type", "create_job"),
                ];
                ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);

                e
            })
    }

    /// Builds data submission job metadata from proving metadata
    fn build_da_metadata(&self, proving_metadata: &ProvingMetadata) -> JobMetadata {
        JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Da(DaMetadata {
                block_number: proving_metadata.block_number,
                blob_data_path: Some(self.build_blob_data_path(proving_metadata.block_number)),
                tx_hash: None, // Will be populated during processing
            }),
        }
    }

    /// Builds the blob data path for a given block number
    fn build_blob_data_path(&self, block_number: u64) -> String {
        format!("{}/{BLOB_DATA_FILE_NAME}", block_number)
    }
}

/// Context for processing jobs, containing shared data and configuration
struct ProcessingContext {
    config: Arc<Config>,
    earliest_failed_block: Option<u64>,
}

impl ProcessingContext {
    /// Creates a new processing context with necessary data fetched
    async fn new(config: Arc<Config>) -> color_eyre::Result<Self> {
        let earliest_failed_block = config.database().get_earliest_failed_block_number().await?;

        Ok(Self { config, earliest_failed_block })
    }

    /// Fetches all proving jobs eligible for data submission job creation
    async fn get_eligible_proving_jobs(&self) -> Result<Vec<JobItem>, DatabaseError> {
        self.config
            .database()
            .get_jobs_without_successor(JobType::ProofCreation, JobStatus::Completed, JobType::DataSubmission)
            .await
    }

    /// Determines if a block number should be skipped due to failed block constraints
    fn should_skip_block(&self, block_number: u64) -> bool {
        match self.earliest_failed_block {
            Some(failed_block) => block_number >= failed_block,
            None => false,
        }
    }
}
