use crate::core::config::Config;
use crate::types::constant::PROOF_FILE_NAME;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::filter_jobs_by_orchestrator_version;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;
use std::sync::Arc;
use tracing::{debug, error, warn};

pub struct ProvingJobTrigger;

#[async_trait]
impl JobTrigger for ProvingJobTrigger {
    /// 1. Fetch all successful SNOS job runs that don't have a proving job
    /// 2. Create a proving job for each SNOS job run
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Self-healing: We intentionally do not heal orphaned Proving jobs as
        // they might create inconsistent state on the atlantic side,
        // sending request twice, opening the same bucket again, adding the the
        // same block again etc.

        let successful_snos_jobs = config
            .database()
            .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
            .await?;

        let successful_snos_jobs = filter_jobs_by_orchestrator_version(successful_snos_jobs);

        debug!("Found {} successful SNOS jobs without proving jobs", successful_snos_jobs.len());

        for snos_job in successful_snos_jobs {
            // Extract SNOS metadata
            let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().map_err(|e| {
                error!(job_id = %snos_job.internal_id, error = %e, "Invalid metadata type for SNOS job");
                e
            })?;

            let (download_proof, snos_fact, bucket_id, bucket_job_index) = match config.layer() {
                Layer::L2 => {
                    // Set the bucket_id and bucket_job_index for Applicative Recursion
                    match config.database().get_aggregator_batch_for_block(snos_metadata.start_block).await? {
                        Some(batch) => {
                            (None, None, Some(batch.bucket_id), Some(snos_metadata.start_block - batch.start_block + 1))
                        }
                        None => {
                            warn!(job_id = %snos_job.internal_id, "No batch found for block {}, skipping for now", snos_metadata.start_block);
                            continue;
                        }
                    }
                }
                Layer::L3 => {
                    // Set the snos_fact and path to download proof
                    let snos_fact = match &snos_metadata.snos_fact {
                        Some(fact) => fact.clone(),
                        None => {
                            error!(job_id = %snos_job.internal_id, "SNOS fact not found in metadata");
                            continue;
                        }
                    };
                    (Some(format!("{}/{}", snos_job.internal_id, PROOF_FILE_NAME)), Some(snos_fact), None, None)
                }
            };

            // Create proving job metadata
            let proving_metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Proving(ProvingMetadata {
                    block_number: snos_metadata.start_block,
                    // Set input path as CairoPie type
                    input_path: snos_metadata.cairo_pie_path.map(ProvingInputType::CairoPie),
                    // Set a download path if needed
                    download_proof,
                    // Set SNOS fact for on-chain verification
                    ensure_on_chain_registration: snos_fact,
                    n_steps: snos_metadata.snos_n_steps,
                    // Set the bucket_id and bucket_job_index for Applicative Recursion
                    bucket_id,
                    bucket_job_index,
                }),
            };

            debug!(job_id = %snos_job.internal_id, "Creating proof creation job for SNOS job");
            match JobHandlerService::create_job(
                JobType::ProofCreation,
                snos_job.internal_id.clone(),
                proving_metadata,
                config.clone(),
            )
            .await
            {
                Ok(_) => {}
                Err(e) => {
                    error!(error = %e, "Failed to create new {:?} job for {}", JobType::ProofCreation, snos_job.internal_id);
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::ProofCreation)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }
}
