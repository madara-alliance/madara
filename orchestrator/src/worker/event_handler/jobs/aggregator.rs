use crate::core::config::Config;
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::BatchStatus;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{AggregatorMetadata, JobMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::get_fact_info;
use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use orchestrator_atlantic_service::constants::{CAIRO_PIE_FILE_NAME, PROOF_FILE_NAME};
use orchestrator_prover_client_interface::{Task, TaskStatus, TaskType};
use starknet_core::types::Felt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct AggregatorJobHandler;

#[async_trait]
impl JobHandlerTrait for AggregatorJobHandler {
    #[tracing::instrument(fields(category = "aggregator"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        info!(
            log_type = "starting",
            category = "aggregator",
            function_type = "create_job",
            block_no = %internal_id,
            "Aggregator job creation started."
        );
        let job_item = JobItem::create(internal_id.clone(), JobType::Aggregator, JobStatus::Created, metadata);
        info!(
            log_type = "completed",
            category = "aggregator",
            function_type = "create_job",
            block_no = %internal_id,
            "Aggregator job creation completed."
        );
        Ok(job_item)
    }

    /// Note: We confirm before creating an Aggregator job that
    /// 1. All its child jobs are completed (i.e., ProofCreation jobs for all the blocks in the batch have status as Completed)
    /// 2. Batch status is Closed (i.e., no new blocks will be added in the batch now)
    ///
    /// So all the Aggregator jobs have the above conditions satisfied.
    /// Now, we follow the following logic:
    /// 1. Call close batch for the bucket
    #[tracing::instrument(skip_all, fields(category = "aggregator", job_id = %job.id, internal_id = %job.internal_id), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        info!(log_type = "starting", "Aggregator job processing started.");

        // Get aggregator metadata
        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        debug!(bucket_id = %metadata.bucket_id, "Closing bucket");

        // Call close bucket
        let external_id =
            config.prover_client().submit_task(Task::CloseBucket(metadata.bucket_id)).await.map_err(|e| {
                error!(error = %e, "Failed to submit close bucket task to prover client");
                JobError::ProverClientError(e)
            })?;

        config
            .database()
            .update_batch_status_by_index(metadata.batch_num, BatchStatus::PendingAggregatorVerification)
            .await?;

        info!(
            log_type = "completed",
            bucket_id = %external_id,
            "Aggregator job processing completed."
        );

        Ok(external_id)
    }

    #[tracing::instrument(skip_all, fields(category = "aggregator", job_id = %job.id, internal_id = %job.internal_id), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        info!(log_type = "starting", "Aggregator job verification started.");

        // Get aggregator metadata
        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        let bucket_id = metadata.bucket_id;

        debug!(
            bucket_id = %bucket_id,
            "Getting bucket status from prover client"
        );

        let task_status =
            config.prover_client().get_task_status(TaskType::Bucket, &bucket_id, None, false).await.map_err(|e| {
                error!(
                    error = %e,
                    "Failed to get bucket status from prover client"
                );
                JobError::Other(OtherError(eyre!("Prover Client Error: {}", e)))
            })?;

        match task_status {
            TaskStatus::Processing => {
                info!("Aggregator job verification pending.");
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // Get the aggregator query ID
                let aggregator_query_id =
                    config.prover_client().get_aggregator_task_id(&bucket_id, metadata.num_blocks + 1).await.map_err(
                        |e| {
                            error!(
                                error = %e,
                                "Failed to get aggregator query ID from prover client"
                            );
                            JobError::Other(OtherError(eyre!(e)))
                        },
                    )?;

                // Fetch aggregator cairo pie and store it in storage
                let cairo_pie_bytes = AggregatorJobHandler::fetch_and_store_artifact(
                    &config,
                    &aggregator_query_id,
                    CAIRO_PIE_FILE_NAME,
                    &metadata.cairo_pie_path,
                )
                .await?;

                // Fetch aggregator snos output and store it in storage
                // TODO: Uncomment this code when atlantic provides the snos output for aggregator jobs
                // AggregatorJobHandler::fetch_and_store_artifact(
                //     &config,
                //     &aggregator_query_id,
                //     SNOS_OUTPUT_FILE_NAME,
                //     &metadata.snos_output_path,
                // )
                // .await?;

                // Calculate the program output from the cairo pie
                let cairo_pie =
                    CairoPie::from_bytes(&cairo_pie_bytes).map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
                let fact_info = get_fact_info(&cairo_pie, None, true)?;
                let program_output = fact_info.program_output;

                // Store the program output in storage
                AggregatorJobHandler::store_program_output(
                    &config,
                    job.internal_id.clone(),
                    program_output,
                    &metadata.program_output_path,
                )
                .await?;

                // TODO: We can check if the fact got registered here only and fail verification if it didn't

                // Download the proof if the path is specified
                if let Some(download_path) = metadata.download_proof {
                    AggregatorJobHandler::fetch_and_store_artifact(
                        &config,
                        &aggregator_query_id,
                        PROOF_FILE_NAME,
                        &download_path,
                    )
                    .await?;
                }

                // Update the batch status to ReadyForStateUpdate
                config
                    .database()
                    .update_batch_status_by_index(metadata.batch_num, BatchStatus::ReadyForStateUpdate)
                    .await?;

                info!("Aggregator job verification completed.");

                // Return the status that the job is verified
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                config
                    .database()
                    .update_batch_status_by_index(metadata.batch_num, BatchStatus::VerificationFailed)
                    .await?;
                warn!("Aggregator job verification failed.");
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
        300
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        300
    }
}

impl AggregatorJobHandler {
    pub async fn fetch_and_store_artifact(
        config: &Arc<Config>,
        task_id: &str,
        file_name: &str,
        storage_path: &str,
    ) -> Result<Vec<u8>, JobError> {
        // TODO: Check if we can optimize the memory usage here
        debug!("Downloading {} and storing to path: {}", file_name, storage_path);
        let artifact = config.prover_client().get_task_artifacts(task_id, file_name).await.map_err(|e| {
            error!(error = %e, "Failed to download {}", file_name);
            JobError::Other(OtherError(eyre!(e)))
        })?;

        config.storage().put_data(bytes::Bytes::from(artifact.clone()), storage_path).await?;

        Ok(artifact)
    }

    pub async fn store_program_output(
        config: &Arc<Config>,
        batch_index: String,
        program_output: Vec<Felt>,
        storage_path: &str,
    ) -> Result<(), SnosError> {
        let program_output: Vec<[u8; 32]> = program_output.iter().map(|f| f.to_bytes_be()).collect();
        let encoded_data = bincode::serialize(&program_output).map_err(|e| SnosError::ProgramOutputUnserializable {
            internal_id: batch_index.clone(),
            message: e.to_string(),
        })?;
        config.storage().put_data(encoded_data.into(), storage_path).await.map_err(|e| {
            SnosError::ProgramOutputUnstorable { internal_id: batch_index.clone(), message: e.to_string() }
        })?;
        Ok(())
    }
}
