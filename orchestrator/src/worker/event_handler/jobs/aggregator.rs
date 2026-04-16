use crate::core::config::{Config, ProverKind};
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::AggregatorBatchStatus;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{AggregatorMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::get_fact_info;
use async_trait::async_trait;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use orchestrator_aggregator_runner::{AggregatorFelt, PROGRAM_HASHES};
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
    /// - **Atlantic**: closes the bucket via `RunAggregation(bucket_id)`.
    /// - **SHARP**: runs the aggregator locally, stores artifacts, then submits an
    ///   applicative job via `RunAggregationWithPie`.
    /// - **Mock**: runs the aggregator locally, stores artifacts, then optionally
    ///   registers the fact hash on an L1 `MockGpsVerifier` via the prover service.
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id;
        info!(log_type = "starting", job_id = %job.id, "{:?} job {} processing started", JobType::Aggregator, internal_id);

        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;

        tracing::Span::current().record("batch_id", metadata.batch_num);
        if let Some(ref bucket_id) = metadata.bucket_id {
            tracing::Span::current().record("bucket_id", bucket_id.as_str());
        }

        let external_id = match config.prover_kind() {
            ProverKind::Atlantic => {
                let bucket_id = metadata
                    .bucket_id
                    .as_ref()
                    .ok_or_else(|| JobError::Other(OtherError(eyre!("Atlantic aggregator job missing bucket_id"))))?;
                config.prover_client().submit_task(Task::RunAggregation(bucket_id.clone())).await.map_err(|e| {
                    error!(error = %e, "Failed to close bucket");
                    JobError::ProverClientError(e)
                })?
            }
            ProverKind::Sharp => self.process_job_sharp(&config, job).await?,
            ProverKind::Mock => self.process_job_local_agg(&config, job).await?,
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

    /// Verify the aggregator job (prover-agnostic).
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id;
        debug!(log_type = "starting", job_id = %job.id, "{:?} job {} verification started", JobType::Aggregator, internal_id);

        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;
        let external_id: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                error!(error = %e, "Failed to unwrap external_id");
                JobError::Other(OtherError(e))
            })?
            .into();

        // Mock uses `ensure_on_chain_registration` to cross-check `isValid(fact)` on the
        // mock verifier during verification. For Atlantic/SHARP this field is either
        // unset or used differently, so passing it through is safe either way.
        let fact = metadata.ensure_on_chain_registration.clone();
        let task_status =
            config.prover_client().get_task_status(TaskType::Aggregation, &external_id, fact, false).await.map_err(
                |e| {
                    error!(error = %e, "Failed to get aggregation status");
                    JobError::Other(OtherError(eyre!("Prover Client Error: {}", e)))
                },
            )?;

        match task_status {
            TaskStatus::Processing => {
                info!(job_id = %job.id, "{:?} job {} verification pending", JobType::Aggregator, internal_id);
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // Atlantic returns bytes; SHARP/Mock return empty (already stored during process_job).
                let artifacts = config
                    .prover_client()
                    .get_aggregation_artifacts(&external_id, metadata.download_proof.is_some())
                    .await
                    .map_err(|e| {
                        error!(error = %e, "Failed to fetch aggregation artifacts");
                        JobError::Other(OtherError(eyre!(e)))
                    })?;

                if let Some(cairo_pie_bytes) = artifacts.cairo_pie {
                    config
                        .storage()
                        .put_data(bytes::Bytes::from(cairo_pie_bytes.clone()), &metadata.cairo_pie_path)
                        .await?;
                    let cairo_pie =
                        CairoPie::from_bytes(&cairo_pie_bytes).map_err(|e| JobError::Other(OtherError(eyre!(e))))?;
                    let fact_info = get_fact_info(&cairo_pie, None, true)?;
                    AggregatorJobHandler::store_program_output(
                        &config,
                        internal_id,
                        fact_info.program_output,
                        &metadata.program_output_path,
                    )
                    .await?;
                }

                if let Some(da_segment_bytes) = artifacts.da_segment {
                    config.storage().put_data(bytes::Bytes::from(da_segment_bytes), &metadata.da_segment_path).await?;
                }

                if let Some(proof_bytes) = artifacts.proof {
                    if let Some(ref download_path) = metadata.download_proof {
                        config.storage().put_data(bytes::Bytes::from(proof_bytes), download_path).await?;
                    }
                }

                config
                    .database()
                    .update_aggregator_batch_status_by_index(
                        metadata.batch_num,
                        AggregatorBatchStatus::ReadyForStateUpdate,
                    )
                    .await?;

                info!(log_type = "completed", job_id = %job.id, "{:?} job {} verification completed", JobType::Aggregator, internal_id);
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
                warn!(log_type = "rejected", job_id = %job.id, "{:?} job {} verification failed: {}", JobType::Aggregator, internal_id, err);
                Ok(JobVerificationStatus::Rejected(format!("Aggregator job #{} failed: {}", internal_id, err)))
            }
        }
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        900
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        30
    }
}

// =============================================================================
// SHARP-specific process_job logic
// =============================================================================

impl AggregatorJobHandler {
    /// Run the aggregator locally and submit the applicative job to SHARP.
    async fn process_job_sharp(&self, config: &Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;
        let batch_num = metadata.batch_num;
        info!(batch_num = %batch_num, "Running local aggregation for SHARP");

        // 1. Fetch program outputs and child job keys from DB + storage.
        let snos_batches = config.database().get_snos_batches_by_aggregator_index(batch_num).await?;

        let mut child_program_outputs = Vec::new();
        let mut child_job_keys = Vec::new();

        for snos_batch in &snos_batches {
            let snos_job = config
                .database()
                .get_job_by_internal_id_and_type(snos_batch.index, &JobType::SnosRun)
                .await?
                .ok_or_else(|| {
                    JobError::Other(OtherError(eyre!("SNOS job not found for batch index {}", snos_batch.index)))
                })?;

            let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().inspect_err(|e| {
                error!(error = %e, "Invalid metadata type for SNOS job");
            })?;

            let output_path = snos_metadata.program_output_path.ok_or_else(|| {
                JobError::Other(OtherError(eyre!("SNOS job {} has no program_output_path", snos_batch.index)))
            })?;

            debug!(snos_batch_index = snos_batch.index, path = %output_path, "Fetching program output");
            let output_bytes = config.storage().get_data(&output_path).await?;
            let program_output: Vec<[u8; 32]> = bincode::deserialize(&output_bytes)
                .map_err(|e| JobError::Other(OtherError(eyre!("Failed to deserialize program output: {}", e))))?;
            child_program_outputs.push(program_output);

            let proving_job = config
                .database()
                .get_job_by_internal_id_and_type(snos_batch.index, &JobType::ProofCreation)
                .await?
                .ok_or_else(|| {
                    JobError::Other(OtherError(eyre!(
                        "ProofCreation job not found for SNOS batch {}",
                        snos_batch.index
                    )))
                })?;

            let job_key: String = proving_job
                .external_id
                .unwrap_string()
                .map_err(|e| {
                    error!(error = %e, "ProofCreation job has invalid external_id");
                    JobError::Other(OtherError(e))
                })?
                .into();
            child_job_keys.push(job_key);
        }

        info!(num_children = child_program_outputs.len(), "Collected program outputs, running aggregator");

        // 2. Run local aggregator.
        let chain_details = config.chain_details();
        let chain_id = AggregatorFelt::from_hex(&format!("0x{}", hex::encode(&chain_details.chain_id)))
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse chain_id: {}", e))))?;
        let fee_token_address = AggregatorFelt::from_hex(chain_details.strk_fee_token_address.as_str())
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse fee_token_address: {}", e))))?;
        let da_public_keys = config
            .params
            .da_public_keys
            .as_ref()
            .map(|keys| keys.iter().map(|f| AggregatorFelt::from_bytes_be(&f.to_bytes_be())).collect::<Vec<_>>());

        let aggregator_output = orchestrator_aggregator_runner::run_local_aggregator(
            orchestrator_aggregator_runner::AggregatorRunnerInput {
                child_program_outputs,
                layout: LayoutName::all_cairo,
                full_output: false,
                debug_mode: false,
                chain_id,
                fee_token_address,
                da_public_keys,
            },
        )
        .map_err(|e| {
            error!(error = %e, "Local aggregator failed");
            JobError::Other(OtherError(eyre!("Aggregator runner failed: {}", e)))
        })?;

        info!("Local aggregator completed, storing artifacts");

        // Use the applicative-bootloader-wrapped aggregator hash, same as the Mock path.
        let fact_info =
            get_fact_info(&aggregator_output.aggregator_cairo_pie, Some(PROGRAM_HASHES.aggregator_with_prefix), true)?;

        // Stamp the fact hex so verify_job can pass it to get_task_status for on-chain
        // cross-verification. Same pattern as the Mock path.
        let mut metadata = metadata;
        metadata.ensure_on_chain_registration = Some(format!("0x{}", hex::encode(fact_info.fact.0)));
        job.metadata.specific = JobSpecificMetadata::Aggregator(metadata.clone());

        AggregatorJobHandler::store_program_output(
            config,
            batch_num,
            fact_info.program_output,
            &metadata.program_output_path,
        )
        .await?;

        config.storage().put_data(bytes::Bytes::from(aggregator_output.da_segment), &metadata.da_segment_path).await?;

        let pie_bytes = crate::worker::utils::pie::cairo_pie_to_zip_bytes(aggregator_output.aggregator_cairo_pie)
            .await
            .map_err(|e| JobError::Other(OtherError(eyre!(e))))?;

        config.storage().put_data(pie_bytes.clone(), &metadata.cairo_pie_path).await?;

        info!(num_children = child_job_keys.len(), "Submitting applicative job to SHARP");
        let fact: [u8; 32] = fact_info.fact.0;
        let external_id = config
            .prover_client()
            .submit_task(Task::RunAggregationWithPie(ApplicativeJobInfo {
                cairo_pie_zip_bytes: pie_bytes,
                children_cairo_job_keys: child_job_keys,
                fact_hash: Some(fact),
            }))
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to submit applicative job");
                JobError::ProverClientError(e)
            })?;

        info!(external_id = %external_id, "Applicative job submitted to SHARP");
        Ok(external_id)
    }
}

// =============================================================================
// Mock-specific local aggregation path
// =============================================================================

impl AggregatorJobHandler {
    /// Run the aggregator locally and submit the resulting PIE to the Mock prover.
    ///
    /// Computes the aggregator PIE's fact hash via `get_fact_info`, stamps the hex
    /// into `metadata.ensure_on_chain_registration` (so `verify_job` can cross-check
    /// `isValid` on-chain), then threads the fact into `ApplicativeJobInfo.fact_hash`
    /// so the mock prover can register it on the L1 `MockGpsVerifier`.
    async fn process_job_local_agg(&self, config: &Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let metadata: AggregatorMetadata = job.metadata.specific.clone().try_into()?;
        let batch_num = metadata.batch_num;
        info!(batch_num = %batch_num, "Running local aggregation for Mock prover");

        // 1. Fetch program outputs and child job keys from DB + storage.
        let snos_batches = config.database().get_snos_batches_by_aggregator_index(batch_num).await?;

        let mut child_program_outputs = Vec::new();
        let mut child_job_keys = Vec::new();

        for snos_batch in &snos_batches {
            let snos_job = config
                .database()
                .get_job_by_internal_id_and_type(snos_batch.index, &JobType::SnosRun)
                .await?
                .ok_or_else(|| {
                    JobError::Other(OtherError(eyre!("SNOS job not found for batch index {}", snos_batch.index)))
                })?;

            let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().inspect_err(|e| {
                error!(error = %e, "Invalid metadata type for SNOS job");
            })?;

            let output_path = snos_metadata.program_output_path.ok_or_else(|| {
                JobError::Other(OtherError(eyre!("SNOS job {} has no program_output_path", snos_batch.index)))
            })?;

            debug!(snos_batch_index = snos_batch.index, path = %output_path, "Fetching program output");
            let output_bytes = config.storage().get_data(&output_path).await?;
            let program_output: Vec<[u8; 32]> = bincode::deserialize(&output_bytes)
                .map_err(|e| JobError::Other(OtherError(eyre!("Failed to deserialize program output: {}", e))))?;
            child_program_outputs.push(program_output);

            let proving_job = config
                .database()
                .get_job_by_internal_id_and_type(snos_batch.index, &JobType::ProofCreation)
                .await?
                .ok_or_else(|| {
                    JobError::Other(OtherError(eyre!(
                        "ProofCreation job not found for SNOS batch {}",
                        snos_batch.index
                    )))
                })?;

            let job_key: String = proving_job
                .external_id
                .unwrap_string()
                .map_err(|e| {
                    error!(error = %e, "ProofCreation job has invalid external_id");
                    JobError::Other(OtherError(e))
                })?
                .into();
            child_job_keys.push(job_key);
        }

        info!(num_children = child_program_outputs.len(), "Collected program outputs, running aggregator");

        // 2. Run local aggregator.
        let chain_details = config.chain_details();
        let chain_id = AggregatorFelt::from_hex(&format!("0x{}", hex::encode(&chain_details.chain_id)))
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse chain_id: {}", e))))?;
        let fee_token_address = AggregatorFelt::from_hex(chain_details.strk_fee_token_address.as_str())
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse fee_token_address: {}", e))))?;
        let da_public_keys = config
            .params
            .da_public_keys
            .as_ref()
            .map(|keys| keys.iter().map(|f| AggregatorFelt::from_bytes_be(&f.to_bytes_be())).collect::<Vec<_>>());

        let aggregator_output = orchestrator_aggregator_runner::run_local_aggregator(
            orchestrator_aggregator_runner::AggregatorRunnerInput {
                child_program_outputs,
                layout: LayoutName::all_cairo,
                full_output: false,
                debug_mode: false,
                chain_id,
                fee_token_address,
                da_public_keys,
            },
        )
        .map_err(|e| {
            error!(error = %e, "Local aggregator failed");
            JobError::Other(OtherError(eyre!("Aggregator runner failed: {}", e)))
        })?;

        info!("Local aggregator completed, storing artifacts");

        // 3. Compute fact info from PIE (program output + fact hash for on-chain registration).
        let fact_info =
            get_fact_info(&aggregator_output.aggregator_cairo_pie, Some(PROGRAM_HASHES.aggregator_with_prefix), true)?;
        let fact: [u8; 32] = fact_info.fact.0;

        // Stamp the fact hex into the aggregator metadata so `verify_job` can pass it to
        // the prover's `get_task_status` for an on-chain `isValid` cross-check.
        //
        // Safe to mutate `job.metadata` in-place: the job framework persists
        // `job.metadata.clone()` immediately after `process_job` returns (see
        // worker/event_handler/service.rs), so this survives into `verify_job`.
        let mut metadata = metadata;
        metadata.ensure_on_chain_registration = Some(format!("0x{}", hex::encode(fact)));
        job.metadata.specific = JobSpecificMetadata::Aggregator(metadata.clone());

        AggregatorJobHandler::store_program_output(
            config,
            batch_num,
            fact_info.program_output,
            &metadata.program_output_path,
        )
        .await?;

        // 4. Store DA segment.
        config.storage().put_data(bytes::Bytes::from(aggregator_output.da_segment), &metadata.da_segment_path).await?;

        // 5. Convert PIE -> zip bytes (consumed/dropped before bytes are buffered).
        let pie_bytes = crate::worker::utils::pie::cairo_pie_to_zip_bytes(aggregator_output.aggregator_cairo_pie)
            .await
            .map_err(|e| JobError::Other(OtherError(eyre!(e))))?;

        // 6. Persist zip to storage.
        config.storage().put_data(pie_bytes.clone(), &metadata.cairo_pie_path).await?;

        // 7. Submit to prover.
        info!(num_children = child_job_keys.len(), "Submitting aggregator PIE to prover");
        let external_id = config
            .prover_client()
            .submit_task(Task::RunAggregationWithPie(ApplicativeJobInfo {
                cairo_pie_zip_bytes: pie_bytes,
                children_cairo_job_keys: child_job_keys,
                fact_hash: Some(fact),
            }))
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to submit aggregator PIE");
                JobError::ProverClientError(e)
            })?;

        info!(external_id = %external_id, "Aggregator PIE submitted");
        Ok(external_id)
    }
}

// =============================================================================
// Shared utilities
// =============================================================================

impl AggregatorJobHandler {
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
