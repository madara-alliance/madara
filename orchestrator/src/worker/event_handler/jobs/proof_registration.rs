use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::constant::{PROOF_FILE_NAME, PROOF_PART2_FILE_NAME};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, ProvingMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::COMPILED_VERIFIER;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use anyhow::Context;
use async_trait::async_trait;
use orchestrator_prover_client_interface::TaskStatus;
use std::sync::Arc;
use swiftness_proof_parser::{parse, StarkProof};

pub struct RegisterProofJobHandler;

#[async_trait]
impl JobHandlerTrait for RegisterProofJobHandler {
    #[tracing::instrument(fields(category = "proof_registry"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(log_type = "starting", category = "proof_registry", function_type = "create_job",  block_no = %internal_id, "Proof Registry job creation started.");
        let job_item = JobItem::create(internal_id.clone(), JobType::ProofRegistration, JobStatus::Created, metadata);
        tracing::info!(log_type = "completed", category = "proving", function_type = "create_job",  block_no = %internal_id, "Proving job created.");
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "proof_registry"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "proof_registry",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "Proof registration job processing started."
        );

        // Get proof from storage
        let proof_key = format!("{internal_id}/{PROOF_FILE_NAME}");
        tracing::debug!(job_id = %job.internal_id, %proof_key, "Fetching proof file");

        let proof_file = config.storage().get_data(&proof_key).await?;

        let proof = String::from_utf8(proof_file.to_vec()).context(format!(
            "Failed to parse proof file as UTF-8 for job_id: {}, proof_key: {}",
            job.internal_id, proof_key
        ))?;

        let _: StarkProof = parse(proof.clone())
            .context(format!("Failed to parse proof file as UTF-8, internal-id: {}", job.internal_id))?;

        // Format proof for submission
        let formatted_proof = format!("{{\n\t\"proof\": {}\n}}", proof);

        let task_id = job.internal_id.clone();

        let proof_verifier = String::from_utf8_lossy(COMPILED_VERIFIER).to_string();
        // Submit proof for L2 verification
        let external_id =
            config.prover_client().submit_l2_query(&task_id, &formatted_proof, None, &proof_verifier).await.context(
                format!(
                    "Failed to submit proof for L2 verification for job_id: {}, task_id: {}",
                    job.internal_id, task_id
                ),
            )?;

        tracing::info!(
            log_type = "completed",
            category = "proof_registry",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            %external_id,
            "Proof registration job processed successfully."
        );
        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "proof_registry"), skip(self, config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "proof_registry",
            function_type = "verify_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "Proof registration job verification started."
        );

        let task_id: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to unwrap external_id for job_id: {}, internal_id: {}: {}",
                    job.id,
                    internal_id,
                    e
                )
            })?
            .into();
        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into()?;
        // Determine if we need on-chain verification
        let (cross_verify, fact) = match &proving_metadata.ensure_on_chain_registration {
            Some(fact_str) => (true, Some(fact_str.clone())),
            None => (false, None),
        };

        tracing::debug!(job_id = %job.internal_id, %task_id, "Getting task status from prover client");
        let task_status =
            config.prover_client().get_task_status(&task_id, fact.clone(), cross_verify).await.context(format!(
                "Failed to get task status from prover client for job_id: {}, task_id: {}",
                job.internal_id, task_id
            ))?;

        match task_status {
            TaskStatus::Processing => {
                tracing::info!(
                    log_type = "pending",
                    category = "proof_registry",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Proof registration job verification pending."
                );
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                let fetched_proof =
                    config.prover_client().get_proof(&task_id, fact.unwrap().as_str()).await.context(format!(
                        "Failed to fetch proof from prover client for job_id: {}, task_id: {}",
                        job.internal_id, task_id
                    ))?;

                let proof_key = format!("{internal_id}/{PROOF_PART2_FILE_NAME}");
                config.storage().put_data(bytes::Bytes::from(fetched_proof.into_bytes()), &proof_key).await?;
                tracing::info!(
                    log_type = "completed",
                    category = "proof_registry",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Proof registration job verification completed."
                );
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                tracing::info!(
                    log_type = "failed",
                    category = "proof_registry",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Proof registration job verification failed."
                );
                Ok(JobVerificationStatus::Rejected(format!(
                    "Proof registration job #{} failed with error: {}",
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
        300
    }
}
