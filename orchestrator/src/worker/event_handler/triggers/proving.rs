use crate::core::config::{Config, ProverKind};
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
use tracing::{debug, error, warn};

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

            let (download_proof, ensure_on_chain_registration, bucket_id, bucket_job_index) = match config.layer() {
                Layer::L2 => {
                    let (bucket_id, bucket_job_index) = match config.prover_kind() {
                        ProverKind::Atlantic => {
                            match config.database().get_aggregator_batch_for_block(snos_metadata.start_block).await? {
                                Some(batch) => {
                                    let bucket_id = match batch.bucket_id.clone() {
                                        Some(bucket_id) => bucket_id,
                                        None => {
                                            warn!(
                                                job_id = snos_job.internal_id,
                                                batch_index = batch.index,
                                                "Atlantic aggregator batch is missing bucket_id. Skipping for now."
                                            );
                                            continue;
                                        }
                                    };

                                    match config.database().get_start_snos_batch_for_aggregator(batch.index).await? {
                                        None => {
                                            warn!(
                                            job_id = snos_job.internal_id,
                                            "Failed to fetch first SNOS job for Aggregator batch {}. Skipping for now.",
                                            batch.index
                                        );
                                            continue;
                                        }
                                        Some(start_snos_batch) => (
                                            Some(bucket_id),
                                            Some(snos_metadata.snos_batch_index - start_snos_batch.index + 1),
                                        ),
                                    }
                                }
                                None => {
                                    warn!(job_id = %snos_job.internal_id, "No batch found for block {}, skipping for now", snos_metadata.start_block);
                                    continue;
                                }
                            }
                        }
                        ProverKind::Sharp | ProverKind::Mock => (None, None),
                    };

                    (
                        if config.params.store_audit_artifacts {
                            Some(format!("{}/{}", snos_job.internal_id, PROOF_FILE_NAME))
                        } else {
                            None
                        },
                        None, // L2 child proofs are not individually cross-checked on-chain.
                        bucket_id,
                        bucket_job_index,
                    )
                }
                Layer::L3 => {
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
                    ensure_on_chain_registration,
                    n_steps: snos_metadata.snos_n_steps,
                    // Set Atlantic bucket metadata for L2 applicative recursion when needed
                    bucket_id,
                    bucket_job_index,
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
