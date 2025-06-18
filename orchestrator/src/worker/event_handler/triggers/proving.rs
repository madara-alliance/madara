use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::Context;
use opentelemetry::KeyValue;

use crate::core::config::Config;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::{JobTrigger, ProcessingResult};

pub struct ProvingJobTrigger;

#[async_trait]
impl JobTrigger for ProvingJobTrigger {
    /// Creates proving jobs for all successful SNOS jobs that don't have one yet.
    ///
    /// This worker:
    /// 1. Fetches all completed SNOS jobs without proving jobs
    /// 2. Validates each job can be processed (not beyond failed blocks)
    /// 3. Creates corresponding proving jobs with proper metadata
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::info!(log_type = "starting", category = "ProvingWorker", "ProvingWorker started.");

        let processing_context = ProcessingContext::new(config.clone()).await?;
        let eligible_snos_jobs = processing_context.get_eligible_snos_jobs().await?;

        tracing::debug!("Found {} successful SNOS jobs without proving jobs", eligible_snos_jobs.len());

        let mut created_jobs = 0;
        let mut skipped_jobs = 0;

        for snos_job in eligible_snos_jobs {
            match self.process_snos_job(&snos_job, &processing_context).await {
                ProcessingResult::Created => created_jobs += 1,
                ProcessingResult::Skipped => skipped_jobs += 1,
                ProcessingResult::Failed => {
                    // Error already logged in process_snos_job
                    continue;
                }
            }
        }

        tracing::info!(created = created_jobs, skipped = skipped_jobs, "ProvingWorker completed job processing");

        Ok(())
    }
}

impl ProvingJobTrigger {
    /// Processes a single SNOS job to create its corresponding proving job
    async fn process_snos_job(&self, snos_job: &JobItem, context: &ProcessingContext) -> ProcessingResult {
        // Extract and validate SNOS metadata
        let snos_metadata = match self.extract_snos_metadata(snos_job) {
            Ok(metadata) => metadata,
            Err(_) => return ProcessingResult::Failed,
        };

        // Check if job should be skipped due to failed block constraints
        if context.should_skip_block(snos_metadata.block_number) {
            tracing::debug!(
                job_id = %snos_job.internal_id,
                block_number = snos_metadata.block_number,
                earliest_failed_block = context.earliest_failed_block,
                "Skipping proving job due to failed block constraint"
            );
            return ProcessingResult::Skipped;
        }

        // Validate SNOS fact availability
        let snos_fact = match self.extract_snos_fact(&snos_metadata, &snos_job.internal_id) {
            Ok(fact) => fact,
            Err(_) => return ProcessingResult::Failed,
        };

        // Create and submit proving job
        match self.create_proving_job(snos_job, &snos_metadata, snos_fact, context.config.clone()).await {
            Ok(_) => {
                tracing::info!(
                    block_number = snos_metadata.block_number,
                    job_id = %snos_job.internal_id,
                    "Successfully created proving job"
                );
                ProcessingResult::Created
            }
            Err(_) => ProcessingResult::Failed,
        }
    }

    /// Extracts and validates SNOS metadata from the job
    fn extract_snos_metadata(&self, snos_job: &JobItem) -> color_eyre::Result<SnosMetadata> {
        snos_job
            .metadata
            .specific
            .clone()
            .try_into()
            .map_err(|e| {
                tracing::error!(
                    job_id = %snos_job.internal_id,
                    error = %e,
                    "Invalid metadata type for SNOS job"
                );
                e
            })
            .context("Extracting Snos Metadata Failed")
    }

    /// Extracts SNOS fact from metadata with proper error handling
    fn extract_snos_fact(&self, snos_metadata: &SnosMetadata, job_id: &str) -> color_eyre::Result<String> {
        snos_metadata.snos_fact.clone().ok_or_else(|| {
            tracing::error!(job_id = %job_id, "SNOS fact not found in metadata");
            color_eyre::eyre::eyre!("SNOS fact missing from metadata")
        })
    }

    /// Creates a proving job with the appropriate metadata
    async fn create_proving_job(
        &self,
        snos_job: &JobItem,
        snos_metadata: &SnosMetadata,
        snos_fact: String,
        config: Arc<Config>,
    ) -> color_eyre::Result<()> {
        let proving_metadata = self.build_proving_metadata(snos_metadata, snos_fact);

        tracing::debug!(
            job_id = %snos_job.internal_id,
            block_number = snos_metadata.block_number,
            "Creating proving job for SNOS job"
        );

        JobHandlerService::create_job(JobType::ProofCreation, snos_job.internal_id.clone(), proving_metadata, config)
            .await
            .map_err(|e| {
                tracing::warn!(
                    job_id = %snos_job.internal_id,
                    error = %e,
                    "Failed to create proving job"
                );

                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::ProofCreation)),
                    KeyValue::new("operation_type", "create_job"),
                ];
                ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);

                e
            })
            .context("Create Proving Job Failed.")
    }

    /// Builds proving job metadata from SNOS metadata
    fn build_proving_metadata(&self, snos_metadata: &SnosMetadata, snos_fact: String) -> JobMetadata {
        JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Proving(ProvingMetadata {
                block_number: snos_metadata.block_number,
                input_path: snos_metadata.cairo_pie_path.as_ref().map(|path| ProvingInputType::CairoPie(path.clone())),
                download_proof: None,
                ensure_on_chain_registration: Some(snos_fact),
                n_steps: snos_metadata.snos_n_steps,
            }),
        }
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

    /// Fetches all SNOS jobs eligible for proving job creation
    async fn get_eligible_snos_jobs(&self) -> color_eyre::Result<Vec<JobItem>> {
        self.config
            .database()
            .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
            .await
            .context("Failed to get Eligible SNOS Jobs")
    }

    /// Determines if a block number should be skipped due to failed block constraints
    fn should_skip_block(&self, block_number: u64) -> bool {
        match self.earliest_failed_block {
            Some(failed_block) => block_number >= failed_block,
            None => false,
        }
    }
}
