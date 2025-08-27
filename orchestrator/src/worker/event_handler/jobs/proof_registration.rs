use crate::core::config::Config;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, ProvingInputType, ProvingMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::JobHandlerTrait;
use anyhow::Context;
use async_trait::async_trait;
use color_eyre::eyre::eyre;
use orchestrator_prover_client_interface::{TaskStatus, TaskType};
use std::sync::Arc;
use swiftness_proof_parser::{parse, StarkProof};
use tracing::{debug, error, info, warn};

pub struct RegisterProofJobHandler;

#[async_trait]
impl JobHandlerTrait for RegisterProofJobHandler {
    #[tracing::instrument(fields(category = "proof_registry"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        info!(log_type = "starting", "Proof Registry job creation started.");
        let job_item = JobItem::create(internal_id.clone(), JobType::ProofRegistration, JobStatus::Created, metadata);
        info!(log_type = "completed", "Proving job created.");
        Ok(job_item)
    }

    #[tracing::instrument(skip_all, fields(category = "proof_registry", job_id = %job.id, internal_id = %job.internal_id), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into().inspect_err(|e| {
            error!(error = %e, "Failed to convert metadata to ProvingMetadata");
        })?;

        info!(log_type = "starting", "Proof registration job processing started.");

        // Get the proof path from input_path
        let proof_key = match proving_metadata.input_path {
            Some(ProvingInputType::Proof(path)) => path,
            Some(ProvingInputType::CairoPie(_)) => {
                return Err(JobError::Other(OtherError(eyre!("Expected Proof input, got CairoPie"))));
            }
            None => return Err(JobError::Other(OtherError(eyre!("Input path not found in job metadata")))),
        };
        debug!(%proof_key, "Fetching proof file");

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

        // Submit proof for L2 verification
        let external_id = config
            .prover_client()
            .submit_l2_query(&task_id, &formatted_proof, proving_metadata.n_steps)
            .await
            .context(format!(
                "Failed to submit proof for L2 verification for job_id: {}, task_id: {}",
                job.internal_id, task_id
            ))?;

        info!(
            log_type = "completed",
            %external_id,
            "Proof registration job processed successfully."
        );
        Ok(external_id)
    }

    #[tracing::instrument(skip_all, fields(category = "proof_registry", job_id = %job.id, internal_id = %job.internal_id), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        info!(log_type = "starting", "Proof registration job verification started.");

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

        debug!(%task_id, "Getting task status from prover client");
        let task_status =
            config.prover_client().get_task_status(TaskType::Job, &task_id, fact.clone(), cross_verify).await.context(
                format!(
                    "Failed to get task status from prover client for job_id: {}, task_id: {}",
                    job.internal_id, task_id
                ),
            )?;

        match task_status {
            TaskStatus::Processing => {
                info!("Proof registration job verification pending.");
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                if let Some(download_path) = &proving_metadata.download_proof {
                    let fetched_proof = config.prover_client().get_proof(&task_id).await.context(format!(
                        "Failed to fetch proof from prover client for job_id: {}, task_id: {}",
                        job.internal_id, task_id
                    ))?;
                    debug!("Downloading and storing bridge proof to path: {}", download_path);
                    config.storage().put_data(bytes::Bytes::from(fetched_proof.into_bytes()), download_path).await?;
                }
                info!("Proof registration job verification completed.");
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                warn!("Proof registration job verification failed.");
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
