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
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::eyre;
use orchestrator_prover_client_interface::{CreateJobInfo, Task, TaskStatus, TaskType};
use std::sync::Arc;
use swiftness_proof_parser::{parse, StarkProof};
use tracing::{debug, error, info, warn};

pub struct RegisterProofJobHandler;

#[async_trait]
impl JobHandlerTrait for RegisterProofJobHandler {
    async fn create_job(&self, internal_id: u64, metadata: JobMetadata) -> Result<JobItem, JobError> {
        debug!(log_type = "starting", "{:?} job {} creation started", JobType::ProofRegistration, internal_id);
        let job_item = JobItem::create(internal_id, JobType::ProofRegistration, JobStatus::Created, metadata);
        debug!(log_type = "completed", "{:?} job {} creation completed", JobType::ProofRegistration, internal_id);
        Ok(job_item)
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id;
        info!(log_type = "starting", job_id = %job.id, " {:?} job {} processing started", JobType::ProofRegistration, internal_id);

        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into().inspect_err(|e| {
            error!(error = %e, "Failed to convert metadata to ProvingMetadata");
        })?;

        let task_id = internal_id.to_string();
        let external_id = match proving_metadata.input_path {
            Some(ProvingInputType::Proof(proof_key)) => {
                debug!(%proof_key, "Fetching proof file");

                let proof_file = config.storage().get_data(&proof_key).await?;

                let proof = String::from_utf8(proof_file.to_vec()).context(format!(
                    "Failed to parse proof file as UTF-8 for job_id: {}, proof_key: {}",
                    internal_id, proof_key
                ))?;

                let _: StarkProof = parse(proof.clone())
                    .context(format!("Failed to parse proof file as UTF-8, internal-id: {}", internal_id))?;

                let formatted_proof = format!("{{\n\t\"proof\": {}\n}}", proof);

                config
                    .prover_client()
                    .submit_l2_query(&task_id, &formatted_proof, proving_metadata.n_steps)
                    .await
                    .inspect_err(|e| {
                        error!(error = %e, "Failed to submit proof for L2 verification for job {}",
                        internal_id);
                    })?
            }
            Some(ProvingInputType::CairoPie(input_path)) => {
                debug!(%input_path, "Fetching Cairo PIE file for L2 proof verification");

                let cairo_pie_file = config.storage().get_data(&input_path).await?;
                let cairo_pie = Box::new(
                    CairoPie::from_bytes(cairo_pie_file.to_vec().as_slice())
                        .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse Cairo PIE file: {}", e))))?,
                );

                config
                    .prover_client()
                    .submit_task(Task::CreateJob(CreateJobInfo {
                        cairo_pie,
                        bucket_id: None,
                        bucket_job_index: None,
                        num_steps: proving_metadata.n_steps,
                        dedup_id: job.id.to_string(),
                    }))
                    .await
                    .inspect_err(|e| {
                        error!(error = %e, "Failed to submit Cairo PIE for L2 proof verification for job {}", internal_id);
                    })?
            }
            None => return Err(JobError::Other(OtherError(eyre!("Input path not found in job metadata")))),
        };

        info!(log_type = "completed", job_id = %job.id, external_id = %external_id, "{:?} job {} processed successfully", JobType::ProofRegistration, internal_id);
        Ok(external_id)
    }

    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id;
        debug!(log_type = "starting", job_id = %job.id, "{:?} job {} verification started", JobType::ProofRegistration, internal_id);

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
        let task_status = config
            .prover_client()
            .get_task_status(TaskType::Job, &task_id, fact.clone(), cross_verify)
            .await
            .inspect_err(|e| {
                error!(
                    error = %e,
                    "Failed to get task status from prover client for job {}",
                    internal_id
                )
            })?;

        match task_status {
            TaskStatus::Processing => {
                info!(
                    job_id = %job.id,
                    "{:?} job {} verification is pending, will be retried in sometime",
                    JobType::ProofRegistration,
                    internal_id
                );
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                if let Some(download_path) = &proving_metadata.download_proof {
                    let fetched_proof = config.prover_client().get_proof(&task_id).await.inspect_err(|e| {
                        error!(error = %e, "Failed to fetch proof from prover client for job {}", internal_id);
                    })?;
                    debug!("Downloading and storing bridge proof to path: {}", download_path);
                    config.storage().put_data(bytes::Bytes::from(fetched_proof.into_bytes()), download_path).await?;
                }
                info!(log_type = "completed", job_id = %job.id, "{:?} job {} verification completed", JobType::ProofRegistration, internal_id);
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                warn!(log_type = "rejected", job_id = %job.id, "{:?} job {} verification failed", JobType::ProofRegistration, internal_id);
                Ok(JobVerificationStatus::Rejected(format!(
                    "Proof registration job #{} failed with error: {}",
                    internal_id, err
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
