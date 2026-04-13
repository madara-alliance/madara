use crate::core::config::{Config, ProverKind};
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::AggregatorBatchStatus;
use crate::types::constant::DA_SEGMENT_FILE_NAME;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{AggregatorMetadata, JobMetadata, ProvingMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::get_fact_info;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use orchestrator_aggregator_runner::AggregatorFelt;
use orchestrator_atlantic_service::constants::{CAIRO_PIE_FILE_NAME, PROOF_FILE_NAME};
use orchestrator_prover_client_interface::{ApplicativeJobInfo, Task, TaskStatus, TaskType};
use starknet_core::types::Felt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct AggregatorJobHandler;

#[async_trait]
impl JobHandlerTrait for AggregatorJobHandler {
    async fn create_job(&self, internal_id: u64, metadata: JobMetadata) -> Result<JobItem, JobError> {
        debug!(log_type = "starting", "{:?} job {} creation started", JobType::Aggregator, internal_id);
        let job_item = JobItem::create(internal_id, JobType::Aggregator, JobStatus::Created, metadata);
        debug!(log_type = "completed", "{:?} job {} creation completed", JobType::Aggregator, internal_id);
        Ok(job_item)
    }

    /// Process the aggregator job.
    ///
    /// For **Atlantic**: closes the bucket and waits for remote aggregation.
    /// For **SHARP**: runs the aggregator locally, stores artifacts, and submits an applicative job.
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id;
        info!(log_type = "starting", job_id = %job.id, " {:?} job {} processing started", JobType::Aggregator, internal_id);

        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        tracing::Span::current().record("batch_id", metadata.batch_num);
        if let Some(ref bucket_id) = metadata.bucket_id {
            tracing::Span::current().record("bucket_id", bucket_id.as_str());
        }

        let external_id = match config.prover_kind() {
            ProverKind::Atlantic => self.process_job_atlantic(&config, &metadata).await?,
            ProverKind::Sharp => self.process_job_sharp(&config, job, metadata.clone()).await?,
        };

        config
            .database()
            .update_aggregator_batch_status_by_index(
                metadata.batch_num,
                AggregatorBatchStatus::PendingAggregatorVerification,
            )
            .await?;

        info!(log_type = "completed", job_id = %job.id, "{:?} job {} processed successfully", JobType::Aggregator, internal_id);

        Ok(external_id)
    }

    /// Verify the aggregator job.
    ///
    /// For **Atlantic**: polls bucket status, fetches artifacts from the prover.
    /// For **SHARP**: polls applicative job status (artifacts already stored locally).
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id;
        debug!(log_type = "starting", job_id = %job.id, "{:?} job {} verification started", JobType::Aggregator, internal_id);

        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        let result = match config.prover_kind() {
            ProverKind::Atlantic => self.verify_job_atlantic(&config, job, &metadata).await?,
            ProverKind::Sharp => self.verify_job_sharp(&config, job, &metadata).await?,
        };

        Ok(result)
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        900 // 7.5 hrs with 30 secs delay
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        30
    }
}

// =============================================================================
// Atlantic-specific flow (existing behavior)
// =============================================================================

impl AggregatorJobHandler {
    /// Atlantic process: close the bucket and return the bucket_id.
    async fn process_job_atlantic(
        &self,
        config: &Arc<Config>,
        metadata: &AggregatorMetadata,
    ) -> Result<String, JobError> {
        let bucket_id = metadata.bucket_id.as_ref().ok_or_else(|| {
            JobError::Other(OtherError(eyre!("Atlantic aggregator job missing bucket_id in metadata")))
        })?;

        debug!(bucket_id = %bucket_id, "Closing bucket (Atlantic)");

        let external_id =
            config.prover_client().submit_task(Task::CloseBucket(bucket_id.clone())).await.map_err(|e| {
                error!(error = %e, "Failed to submit close bucket task to prover client");
                JobError::ProverClientError(e)
            })?;

        Ok(external_id)
    }

    /// Atlantic verify: poll bucket status, fetch artifacts from the prover.
    async fn verify_job_atlantic(
        &self,
        config: &Arc<Config>,
        job: &mut JobItem,
        metadata: &AggregatorMetadata,
    ) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id;
        let bucket_id = metadata.bucket_id.as_ref().ok_or_else(|| {
            JobError::Other(OtherError(eyre!("Atlantic aggregator job missing bucket_id in metadata")))
        })?;

        debug!(bucket_id = %bucket_id, "Getting bucket status from prover client (Atlantic)");

        let task_status =
            config.prover_client().get_task_status(TaskType::Bucket, bucket_id, None, false).await.map_err(|e| {
                error!(error = %e, "Failed to get bucket status from prover client");
                JobError::Other(OtherError(eyre!("Prover Client Error: {}", e)))
            })?;

        match task_status {
            TaskStatus::Processing => {
                info!(job_id = %job.id, "{:?} job {} verification is pending, will be retried", JobType::Aggregator, internal_id);
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // Get the aggregator query ID
                let aggregator_query_id =
                    config.prover_client().get_aggregator_task_id(bucket_id).await.map_err(|e| {
                        error!(error = %e, "Failed to get aggregator query ID from prover client");
                        JobError::Other(OtherError(eyre!(e)))
                    })?;

                tracing::Span::current().record("aggregator_query_id", aggregator_query_id.as_str());

                // Fetch aggregator cairo pie and store it in storage
                let cairo_pie_bytes = AggregatorJobHandler::fetch_and_store_artifact(
                    config,
                    &aggregator_query_id,
                    CAIRO_PIE_FILE_NAME,
                    &metadata.cairo_pie_path,
                )
                .await?;

                // Fetch DA segment from prover and store it in storage
                AggregatorJobHandler::fetch_and_store_artifact(
                    config,
                    &aggregator_query_id,
                    DA_SEGMENT_FILE_NAME,
                    &metadata.da_segment_path,
                )
                .await?;

                // Calculate the program output from the cairo pie
                let cairo_pie =
                    CairoPie::from_bytes(&cairo_pie_bytes).map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
                let fact_info = get_fact_info(&cairo_pie, None, true)?;
                let program_output = fact_info.program_output;

                // Store the program output in storage
                AggregatorJobHandler::store_program_output(
                    config,
                    internal_id,
                    program_output,
                    &metadata.program_output_path,
                )
                .await?;

                // Download the proof if the path is specified
                if let Some(download_path) = &metadata.download_proof {
                    AggregatorJobHandler::fetch_and_store_artifact(
                        config,
                        &aggregator_query_id,
                        PROOF_FILE_NAME,
                        download_path,
                    )
                    .await?;
                }

                // Update the batch status to ReadyForStateUpdate
                config
                    .database()
                    .update_aggregator_batch_status_by_index(
                        metadata.batch_num,
                        AggregatorBatchStatus::ReadyForStateUpdate,
                    )
                    .await?;

                info!(log_type = "completed", job_id = %job.id, "{:?} job {} verification completed (Atlantic)", JobType::Aggregator, internal_id);
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                config
                    .database()
                    .update_aggregator_batch_status_by_index(
                        metadata.batch_num,
                        AggregatorBatchStatus::VerificationFailed,
                    )
                    .await?;
                warn!(log_type = "rejected", job_id = %job.id, "{:?} job {} verification failed (Atlantic)", JobType::Aggregator, internal_id);
                Ok(JobVerificationStatus::Rejected(format!(
                    "Aggregator job #{} failed with error: {}",
                    internal_id, err
                )))
            }
        }
    }
}

// =============================================================================
// SHARP-specific flow (new)
// =============================================================================

impl AggregatorJobHandler {
    /// SHARP process: run aggregator locally, store artifacts, submit applicative job.
    async fn process_job_sharp(
        &self,
        config: &Arc<Config>,
        _job: &mut JobItem,
        metadata: AggregatorMetadata,
    ) -> Result<String, JobError> {
        let batch_num = metadata.batch_num;
        info!(batch_num = %batch_num, "Processing aggregator job via SHARP (local aggregation)");

        // 1. Fetch all child CairoPIEs and their job keys from completed ProofCreation jobs
        let snos_batches = config.database().get_snos_batches_by_aggregator_index(batch_num).await?;

        let mut child_pies = Vec::new();
        let mut child_job_keys = Vec::new();

        for snos_batch in &snos_batches {
            // Get the proving job for this SNOS batch
            let proving_job = config
                .database()
                .get_job_by_internal_id_and_type(snos_batch.index, &JobType::ProofCreation)
                .await?
                .ok_or_else(|| {
                    JobError::Other(OtherError(eyre!(
                        "ProofCreation job not found for SNOS batch index {}",
                        snos_batch.index
                    )))
                })?;

            // Get child cairo_job_key from external_id (set when the job was submitted to SHARP)
            let job_key: String = proving_job
                .external_id
                .unwrap_string()
                .map_err(|e| {
                    error!(error = %e, "Failed to get SHARP job key from ProofCreation job external_id");
                    JobError::Other(OtherError(e))
                })?
                .into();
            child_job_keys.push(job_key);

            // Get the CairoPIE path from proving metadata
            let proving_metadata: ProvingMetadata = proving_job.metadata.specific.try_into().inspect_err(|e| {
                error!(error = %e, "Invalid metadata type for ProofCreation job");
            })?;

            let cairo_pie_path = match proving_metadata.input_path {
                Some(crate::types::jobs::metadata::ProvingInputType::CairoPie(path)) => path,
                _ => {
                    return Err(JobError::Other(OtherError(eyre!(
                        "ProofCreation job {} has no CairoPie input path",
                        snos_batch.index
                    ))));
                }
            };

            // Fetch CairoPIE from storage
            let pie_bytes = config.storage().get_data(&cairo_pie_path).await?;
            let cairo_pie = CairoPie::from_bytes(&pie_bytes).map_err(|e| {
                error!(error = %e, "Failed to parse CairoPIE for SNOS batch {}", snos_batch.index);
                JobError::Other(OtherError(eyre!(e)))
            })?;
            child_pies.push(cairo_pie);
        }

        info!(
            num_children = child_pies.len(),
            "Collected {} child CairoPIEs, running local aggregator",
            child_pies.len()
        );

        // 2. Run local aggregator
        let chain_details = config.chain_details();
        let chain_id = AggregatorFelt::from_hex(&format!("0x{}", hex::encode(&chain_details.chain_id)))
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse chain_id: {}", e))))?;
        let fee_token_address = AggregatorFelt::from_hex(chain_details.strk_fee_token_address.as_str())
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse fee_token_address: {}", e))))?;

        // Convert starknet_core::types::Felt -> starknet_types_core::felt::Felt for da_public_keys
        let da_public_keys = config
            .params
            .da_public_keys
            .as_ref()
            .map(|keys| keys.iter().map(|f| AggregatorFelt::from_bytes_be(&f.to_bytes_be())).collect::<Vec<_>>());

        let aggregator_output = orchestrator_aggregator_runner::run_local_aggregator(
            orchestrator_aggregator_runner::AggregatorRunnerInput {
                child_cairo_pies: child_pies,
                layout: LayoutName::all_cairo,
                full_output: false,
                debug_mode: false,
                chain_id,
                fee_token_address,
                da_public_keys,
            },
        )
        .map_err(|e| {
            error!(error = %e, "Local aggregator execution failed");
            JobError::Other(OtherError(eyre!("Aggregator runner failed: {}", e)))
        })?;

        info!("Local aggregator completed, storing artifacts");

        // 3. Store aggregator CairoPIE in storage
        let pie_temp = tempfile::NamedTempFile::new().map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
        aggregator_output
            .aggregator_cairo_pie
            .write_zip_file(pie_temp.path(), true)
            .map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
        let pie_bytes = std::fs::read(pie_temp.path()).map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
        config.storage().put_data(bytes::Bytes::from(pie_bytes.clone()), &metadata.cairo_pie_path).await?;

        // 4. Store DA segment in storage
        config.storage().put_data(bytes::Bytes::from(aggregator_output.da_segment), &metadata.da_segment_path).await?;

        // 5. Compute and store program output from the aggregator CairoPIE
        let agg_pie = CairoPie::from_bytes(&pie_bytes).map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
        let fact_info = get_fact_info(&agg_pie, None, true)?;
        AggregatorJobHandler::store_program_output(
            config,
            batch_num,
            fact_info.program_output,
            &metadata.program_output_path,
        )
        .await?;

        // 6. Submit applicative job to SHARP
        info!(num_children = child_job_keys.len(), "Submitting applicative job to SHARP");
        let applicative_key = config
            .prover_client()
            .submit_task(Task::SubmitApplicativeJob(ApplicativeJobInfo {
                cairo_pie: Box::new(aggregator_output.aggregator_cairo_pie),
                children_cairo_job_keys: child_job_keys,
            }))
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to submit applicative job to SHARP");
                JobError::ProverClientError(e)
            })?;

        info!(applicative_job_key = %applicative_key, "Applicative job submitted to SHARP");

        // The applicative_key is returned and set as job.external_id by the framework
        Ok(applicative_key)
    }

    /// SHARP verify: poll the applicative job status.
    /// Artifacts were already stored locally during process_job.
    async fn verify_job_sharp(
        &self,
        config: &Arc<Config>,
        job: &mut JobItem,
        metadata: &AggregatorMetadata,
    ) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id;

        // The applicative job key is stored as external_id by the framework
        // (set from the return value of process_job_sharp)
        let applicative_key: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                error!(error = %e, "Failed to get SHARP applicative job key from external_id");
                JobError::Other(OtherError(e))
            })?
            .into();

        debug!(
            applicative_job_key = %applicative_key,
            "Checking SHARP applicative job status"
        );

        let task_status = config
            .prover_client()
            .get_task_status(TaskType::ApplicativeJob, &applicative_key, None, false)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to get applicative job status from SHARP");
                JobError::Other(OtherError(eyre!("Prover Client Error: {}", e)))
            })?;

        match task_status {
            TaskStatus::Processing => {
                info!(
                    job_id = %job.id,
                    "{:?} job {} SHARP applicative job still processing",
                    JobType::Aggregator, internal_id
                );
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // Artifacts already stored during process_job. Update batch status.
                config
                    .database()
                    .update_aggregator_batch_status_by_index(
                        metadata.batch_num,
                        AggregatorBatchStatus::ReadyForStateUpdate,
                    )
                    .await?;

                info!(
                    log_type = "completed",
                    job_id = %job.id,
                    "{:?} job {} verification completed (SHARP)",
                    JobType::Aggregator, internal_id
                );
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                config
                    .database()
                    .update_aggregator_batch_status_by_index(
                        metadata.batch_num,
                        AggregatorBatchStatus::VerificationFailed,
                    )
                    .await?;
                warn!(
                    log_type = "rejected",
                    job_id = %job.id,
                    "{:?} job {} SHARP applicative job failed: {}",
                    JobType::Aggregator, internal_id, err
                );
                Ok(JobVerificationStatus::Rejected(format!(
                    "SHARP applicative job failed for aggregator #{}: {}",
                    internal_id, err
                )))
            }
        }
    }
}

// =============================================================================
// Shared utilities
// =============================================================================

impl AggregatorJobHandler {
    pub async fn fetch_and_store_artifact(
        config: &Arc<Config>,
        task_id: &str,
        file_name: &str,
        storage_path: &str,
    ) -> Result<Vec<u8>, JobError> {
        debug!("Downloading {} and storing to path: {} for id {}", file_name, storage_path, task_id);
        let artifact = config.prover_client().get_task_artifacts(task_id, file_name).await.map_err(|e| {
            error!(error = %e, "Failed to download {}", file_name);
            JobError::Other(OtherError(eyre!(e)))
        })?;

        config.storage().put_data(bytes::Bytes::from(artifact.clone()), storage_path).await?;

        Ok(artifact)
    }

    pub async fn store_program_output(
        config: &Arc<Config>,
        batch_index: u64,
        program_output: Vec<Felt>,
        storage_path: &str,
    ) -> Result<(), SnosError> {
        let program_output: Vec<[u8; 32]> = program_output.iter().map(|f| f.to_bytes_be()).collect();
        let encoded_data = bincode::serialize(&program_output)
            .map_err(|e| SnosError::ProgramOutputUnserializable { internal_id: batch_index, message: e.to_string() })?;
        config
            .storage()
            .put_data(encoded_data.into(), storage_path)
            .await
            .map_err(|e| SnosError::ProgramOutputUnstorable { internal_id: batch_index, message: e.to_string() })?;
        Ok(())
    }
}
