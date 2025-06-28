use crate::core::client::storage::StorageError;
use crate::core::config::Config;
use crate::core::StorageClient;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::BatchStatus;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{AggregatorMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata};
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
use orchestrator_prover_client_interface::{AtlanticStatusType, Task, TaskStatus, TaskType};
use prove_block::prove_block;
use starknet_os::io::output::StarknetOsOutput;
use std::fs::metadata;
use std::io::Read;
use std::sync::Arc;
use starknet_core::types::Felt;
use tempfile::NamedTempFile;
use crate::error::job::snos::SnosError;

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
        let job_item = JobItem::create(internal_id.clone(), JobType::Aggregator, JobStatus::Created, metadata);
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
        /// 1. Calculate the fact for the batch and save it in job metadata
        /// 2. Call close batch for the bucket
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
            .submit_task(Task::CloseBucket(metadata.bucket_id), None)
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to submit close bucket task to prover client");
                JobError::Other(OtherError(e)) // TODO: Add a new error type to be used for prover client error
            })?;

        config.database().update_batch_status_by_index(metadata.batch_num, BatchStatus::PendingAggregatorVerification).await?;

        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "aggregator"), skip(self, config), ret, err)]
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

        // Get task ID from external_id
        let task_id: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to unwrap external_id");
                JobError::Other(OtherError(e))
            })?
            .into();

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
                // Download the following from the prover client and store in storage
                // - cairo pie
                // - snos output
                // Calculate the program output from these and store this as well in storage
                // If the proof download path is specified, download this as well

                let cairo_pie_string = AggregatorJobHandler::fetch_and_store_artifact(&config, &task_id, "pie.cairo0.zip", &metadata.cairo_pie_path).await?;
                // AggregatorJobHandler::fetch_and_store_artifact(config, &task_id, "snos_output.json", &metadata.snos_output_path).await?;

                let cairo_pie = CairoPie::from_bytes(cairo_pie_string.as_bytes()).map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
                let fact_info = get_fact_info(&cairo_pie, None)?;
                let program_output = fact_info.program_output;

                AggregatorJobHandler::store_program_output(&config, job.internal_id.clone(), program_output, &metadata.program_output_path).await?;

                // TODO: We can check if the fact got registered here only and fail verification if it didn't

                if let Some(download_path) = metadata.download_proof {
                    AggregatorJobHandler::fetch_and_store_artifact(&config, &task_id, "proof.json", &download_path).await?;
                }

                config
                    .database()
                    .update_batch_status_by_index(metadata.batch_num, BatchStatus::ReadyForStateUpdate)
                    .await?;

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
                config
                    .database()
                    .update_batch_status_by_index(metadata.batch_num, BatchStatus::VerificationFailed)
                    .await?;
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
        2
    }

    fn max_verification_attempts(&self) -> u64 {
        300
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        30
    }

    fn job_processing_lock(&self, _config: Arc<Config>) -> Option<Arc<JobProcessingState>> {
        None
    }
}

impl AggregatorJobHandler {
    pub async fn fetch_and_store_artifact(
        config: &Arc<Config>,
        task_id: &str,
        file_name: &str,
        storage_path: &str,
    ) -> Result<String, JobError> {
        tracing::debug!("Downloading {} and storing to path: {}", file_name, storage_path);
        let cairo_pie =
            config.prover_client().get_task_artifacts(&task_id, TaskType::Query, file_name).await.map_err(|e| {
                tracing::error!(error = %e, "Failed to download {}", file_name);
                JobError::Other(OtherError(eyre!(e)))
            })?;

        config.storage().put_data(bytes::Bytes::from(cairo_pie.clone().into_bytes()), storage_path).await?;
        Ok(cairo_pie)
    }

    pub async fn store_program_output(config: &Arc<Config>, batch_index: String, program_output: Vec<Felt>, storage_path: &str) -> Result<(), SnosError> {
        let program_output: Vec<[u8; 32]> = program_output.iter().map(|f| f.to_bytes_be()).collect();
        let encoded_data = bincode::serialize(&program_output).map_err(|e| {
            SnosError::ProgramOutputUnserializable { internal_id: batch_index.clone(), message: e.to_string() }
        })?;
        config.storage().put_data(encoded_data.into(), storage_path).await.map_err(|e| {
            SnosError::ProgramOutputUnstorable { internal_id: batch_index.clone(), message: e.to_string() }
        })?;
        Ok(())
    }
}
