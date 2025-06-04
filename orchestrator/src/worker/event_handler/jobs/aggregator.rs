use crate::core::config::Config;
use crate::core::StorageClient;
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    AggregatorMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::helpers::JobProcessingState;
use crate::utils::COMPILED_OS;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::get_fact_info;
use async_trait::async_trait;
use bytes::Bytes;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use color_eyre::eyre::{eyre, Context};
use color_eyre::Result;
use orchestrator_prover_client_interface::{AtlanticStatusType, Task, TaskStatus};
use prove_block::prove_block;
use starknet_os::io::output::StarknetOsOutput;
use std::io::Read;
use std::sync::Arc;
use tempfile::NamedTempFile;

pub struct AggregatorJobHandler;

#[async_trait]
impl JobHandlerTrait for AggregatorJobHandler {
    #[tracing::instrument(fields(category = "aggregator"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "aggregator",
            function_type = "create_job",
            block_no = %internal_id,
            "Aggregator job creation started."
        );
        let job_item = JobItem::create(internal_id.clone(), JobType::SnosRun, JobStatus::Created, metadata);
        tracing::info!(
            log_type = "completed",
            category = "aggregator",
            function_type = "create_job",
            block_no = %internal_id,
            "Aggregator job creation completed."
        );
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "aggregator"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        /// Note: We confirm before creating an Aggregator job that
        /// 1. All its child jobs are completed (i.e., ProofCreation jobs for all the blocks in the batch have status as Completed)
        /// 2. Batch status is Closed (i.e., no new blocks will be added in the batch now)
        ///
        /// So all the Aggregator jobs have the above conditions satisfied.
        /// Now, we follow the following logic:
        /// 1. Calculate fact for batch
        /// 2. Call close batch for the bucket
        // TODO: add logic to calculate fact for the batch and add it in aggregator metadata
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "aggregator",
            function_type = "process_job",
            job_id = ?job.id,
            batch_no = %internal_id,
            "Aggregator job processing started."
        );

        // Get aggregator metadata
        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        tracing::debug!(batch_no = %internal_id, id = %job.id, bucket_id = %metadata.bucket_id, "Closing bucket");

        // Call close bucket
        let external_id = config
            .prover_client()
            .submit_task(Task::CloseBucket(metadata.bucket_id), None, None, None)
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to submit close bucket task to prover client");
                JobError::Other(OtherError(e)) // TODO: Add a new error type to be used for prover client error
            })?;

        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "aggregator"), skip(self, _config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "aggregator",
            function_type = "verify_job",
            job_id = ?job.id,
            batch_no = %internal_id,
            "Aggregator job verification started."
        );

        // Get aggregator metadata
        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        let bucket_id = metadata.bucket_id;

        tracing::debug!(
            job_id = %job.internal_id,
            bucket_id = %bucket_id,
            "Getting bucket status from prover client"
        );

        let (cross_verify, fact) = match &metadata.ensure_on_chain_registration {
            Some(fact_str) => (true, Some(fact_str.clone())),
            None => (false, None),
        };

        let task_status = config
            .prover_client()
            .get_task_status(AtlanticStatusType::Bucket, &bucket_id, fact, cross_verify)
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| {
                tracing::error!(
                    job_id = %job.internal_id,
                    error = %e,
                    "Failed to get bucket status from prover client"
                );
                JobError::Other(OtherError(e))
            })?;

        match task_status {
            TaskStatus::Processing => {
                tracing::info!(
                    log_type = "pending",
                    category = "proving",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    batch_no = %internal_id,
                    "Aggregator job verification pending."
                );
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // If proof download path is specified, store the proof
                if let Some(download_path) = metadata.download_proof {
                    tracing::debug!(
                        job_id = %job.internal_id,
                        "Downloading and storing proof to path: {}",
                        download_path
                    );
                    // TODO: Implement proof download and storage
                }

                tracing::info!(
                    log_type = "completed",
                    category = "aggregator",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    batch_no = %internal_id,
                    "Aggregator job verification completed."
                );
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                tracing::info!(
                    log_type = "failed",
                    category = "aggregator",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Aggregator job verification failed."
                );
                Ok(JobVerificationStatus::Rejected(format!(
                    "Aggregator job #{} failed with error: {}",
                    job.internal_id, err
                )))
            }
        }
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        1
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        1
    }

    fn job_processing_lock(&self, config: Arc<Config>) -> Option<Arc<JobProcessingState>> {
        todo!()
        // config.processing_locks().snos_job_processing_lock.clone()
    }
}
