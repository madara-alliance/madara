use crate::core::config::Config;
use crate::types::constant::{ORCHESTRATOR_VERSION, PROOF_FILE_NAME};
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::{first_unsettled_snos_batch_index_or_zero, JobTrigger};
use async_trait::async_trait;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;
use std::sync::Arc;
use tracing::{debug, error};

pub struct ProvingJobTrigger;

#[async_trait]
impl JobTrigger for ProvingJobTrigger {
    /// 1. Fetch all successful SNOS job runs that don't have a proving job
    /// 2. Create a proving job for each SNOS job run
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        let min_snos_batch_index_or_zero = first_unsettled_snos_batch_index_or_zero(&config).await?;
        let successful_snos_jobs = config
            .database()
            .get_jobs_without_successor(
                JobType::SnosRun,
                JobStatus::Completed,
                JobType::ProofCreation,
                Some(ORCHESTRATOR_VERSION.to_string()),
                min_snos_batch_index_or_zero,
            )
            .await?;

        debug!("Found {} successful SNOS jobs without proving jobs", successful_snos_jobs.len());

        for snos_job in successful_snos_jobs {
            // Extract SNOS metadata
            let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().map_err(|e| {
                error!(job_id = %snos_job.internal_id, error = %e, "Invalid metadata type for SNOS job");
                e
            })?;

            let (download_proof, ensure_on_chain_registration) = match config.layer() {
                Layer::L2 => (
                    if config.params.store_audit_artifacts {
                        Some(format!("{}/{}", snos_job.internal_id, PROOF_FILE_NAME))
                    } else {
                        None
                    },
                    None, // L2 child proofs are not individually cross-checked on-chain.
                ),
                Layer::L3 => {
                    let snos_fact = match &snos_metadata.snos_fact {
                        Some(fact) => fact.clone(),
                        None => {
                            error!(job_id = %snos_job.internal_id, "SNOS fact not found in metadata");
                            continue;
                        }
                    };
                    (Some(format!("{}/{}", snos_job.internal_id, PROOF_FILE_NAME)), Some(snos_fact))
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
                    ensure_on_chain_registration,
                    n_steps: snos_metadata.snos_n_steps,
                }),
            };

            debug!(job_id = %snos_job.internal_id, "Creating proof creation job for SNOS job");
            match JobHandlerService::create_job(
                JobType::ProofCreation,
                snos_job.internal_id,
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
                    MetricsRecorder::record_failed_job_operation(1.0, &attributes);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }
}
