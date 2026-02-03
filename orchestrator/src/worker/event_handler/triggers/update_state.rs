use crate::core::config::Config;
use crate::types::constant::ORCHESTRATOR_VERSION;
use crate::types::jobs::metadata::{
    AggregatorMetadata, CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, SettlementContext,
    SettlementContextData, SnosMetadata, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use color_eyre::eyre::eyre;

use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct UpdateStateJobTrigger;

#[async_trait]
impl JobTrigger for UpdateStateJobTrigger {
    /// This will create new StateTransition jobs for applicable batches
    /// 1. Get the last batch for which state transition happened
    /// 2. Get all the parent jobs after that last block/batch that have status as Completed
    /// 3. Sanitize and Trim this list of batches
    /// 4. Create a StateTransition job for doing state transitions for all the batches in this list
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        let parent_job_type = match config.layer() {
            Layer::L2 => JobType::Aggregator,
            Layer::L3 => JobType::DataSubmission,
        };

        // Get the latest StateTransition job
        let latest_job = config.database().get_latest_job_by_type(JobType::StateTransition, None).await?;
        // Get the parent jobs that are completed and are ready to get their StateTransition job created
        let (jobs_to_process, last_processed) = match latest_job {
            Some(job) => {
                if job.status != JobStatus::Completed {
                    // If we already have a StateTransition job which is not completed, don't start a new job as it can cause issues
                    info!(
                        "{:?} jobs need to be processed sequentially. {} job is pending. \
                        Parallel jobs can cause nonce issues or can  completely fail. Returning safely.",
                        JobType::StateTransition,
                        job.internal_id
                    );
                    return Ok(());
                }

                // Extract blocks/batches from state transition metadata
                let state_update_metadata: StateUpdateMetadata = job.metadata.specific.try_into().map_err(|e| {
                    error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for state transition job");
                    e
                })?;

                let last_processed = match state_update_metadata.context {
                    SettlementContext::Block(block) => block.to_settle,
                    SettlementContext::Batch(batch) => batch.to_settle,
                };

                (
                    config
                        .database()
                        .get_jobs_after_internal_id_by_job_type(
                            parent_job_type,
                            JobStatus::Completed,
                            last_processed,
                            Some(ORCHESTRATOR_VERSION.to_string()),
                        )
                        .await?,
                    Some(last_processed),
                )
            }
            None => {
                warn!("No previous state transition job found, fetching latest parent job");
                // Getting the latest parent job in case no latest state update job is present
                (
                    config
                        .database()
                        .get_jobs_without_successor(
                            parent_job_type,
                            JobStatus::Completed,
                            JobType::StateTransition,
                            Some(ORCHESTRATOR_VERSION.to_string()),
                        )
                        .await?,
                    None,
                )
            }
        };

        let mut to_process: Vec<u64> = jobs_to_process.iter().map(|j| j.internal_id).collect();
        to_process.sort();

        // no parent jobs completed after the last settled block
        if to_process.is_empty() {
            info!(
                "No parent job of {:?} completed after the last settled batch ({:?}). Returning safely.",
                JobType::StateTransition,
                last_processed
            );
            return Ok(());
        }

        // Verify batch continuity
        match last_processed {
            Some(last_batch) => {
                if to_process[0] != last_batch + 1 {
                    // Keeping the info log here since it can be confusing for the user why state transition job is not being created
                    info!("{:?} jobs need to be processed sequentially. Parent job for {} is not yet completed, hence not creating corresponding {:?} job. Returning safely.", JobType::StateTransition, last_batch + 1, JobType::StateTransition);
                    return Ok(());
                }
            }
            None => {
                // if the last processed batch is not there, (i.e., this is the first StateTransition job), check if the batch being processed is equal to 1
                if to_process[0] != 1 {
                    info!("{:?} jobs need to be processed sequentially. Parent job for {} is not yet completed, hence not creating corresponding {:?} job. Returning safely.", JobType::StateTransition, 1, JobType::StateTransition);
                    return Ok(());
                }
            }
        }

        // Process exactly one block/batch per job - take only the first item
        let batch_or_block_number = to_process[0];

        // Getting settlement context
        let settlement_context =
            SettlementContext::Batch(SettlementContextData { to_settle: batch_or_block_number, last_failed: None });

        // Prepare state transition metadata
        let mut state_update_metadata = StateUpdateMetadata { context: settlement_context, ..Default::default() };

        // Collect paths for the following - snos output, program output and blob data
        match config.layer() {
            Layer::L2 => self.collect_paths_l2(&config, batch_or_block_number, &mut state_update_metadata).await?,
            Layer::L3 => self.collect_paths_l3(&config, batch_or_block_number, &mut state_update_metadata).await?,
        }

        // Create StateTransition job metadata
        let metadata = JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::StateUpdate(state_update_metadata),
        };

        // Create the state transition job
        let new_job_id = batch_or_block_number;
        match JobHandlerService::create_job(JobType::StateTransition, new_job_id, metadata, config.clone()).await {
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, "Failed to create new {:?} job for {}", JobType::StateTransition, new_job_id);
                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::StateTransition)),
                    KeyValue::new("operation_type", format!("{:?}", "create_job")),
                ];
                MetricsRecorder::record_failed_job_operation(1.0, &attributes);
                return Err(e.into());
            }
        }

        Ok(())
    }
}

impl UpdateStateJobTrigger {
    async fn collect_paths_l3(
        &self,
        config: &Arc<Config>,
        block_number: u64,
        state_metadata: &mut StateUpdateMetadata,
    ) -> color_eyre::Result<()> {
        // Get SNOS job paths
        let snos_job = config
            .database()
            .get_job_by_internal_id_and_type(block_number, &JobType::SnosRun)
            .await?
            .ok_or_else(|| eyre!("SNOS job not found for block {}", block_number))?;
        let snos_metadata: SnosMetadata = snos_job.metadata.specific.try_into().map_err(|e| {
            error!(job_id = %snos_job.internal_id, error = %e, "Invalid metadata type for SNOS job");
            e
        })?;

        state_metadata.snos_output_path = snos_metadata.snos_output_path.clone();
        state_metadata.program_output_path = snos_metadata.program_output_path.clone();

        // Get DA job blob path
        let da_job = config
            .database()
            .get_job_by_internal_id_and_type(block_number, &JobType::DataSubmission)
            .await?
            .ok_or_else(|| eyre!("DA job not found for block {}", block_number))?;

        let da_metadata: DaMetadata = da_job.metadata.specific.try_into().map_err(|e| {
            error!(job_id = %da_job.internal_id, error = %e, "Invalid metadata type for DA job");
            e
        })?;

        state_metadata.blob_data_path = da_metadata.blob_data_path.clone();

        Ok(())
    }

    async fn collect_paths_l2(
        &self,
        config: &Arc<Config>,
        batch_no: u64,
        state_metadata: &mut StateUpdateMetadata,
    ) -> color_eyre::Result<()> {
        // Get the aggregator job metadata for the batch
        let aggregator_job = config
            .database()
            .get_job_by_internal_id_and_type(batch_no, &JobType::Aggregator)
            .await?
            .ok_or_else(|| eyre!("Aggregator job not found for batch {}", batch_no))?;
        let aggregator_metadata: AggregatorMetadata = aggregator_job.metadata.specific.try_into().map_err(|e| {
            error!(job_id = %aggregator_job.internal_id, error = %e, "Invalid metadata type for Aggregator job");
            e
        })?;

        // Set the snos output path, program output path, blob data path, and da segment path in state transition metadata
        state_metadata.snos_output_path = Some(aggregator_metadata.snos_output_path.clone());
        state_metadata.program_output_path = Some(aggregator_metadata.program_output_path.clone());
        state_metadata.blob_data_path = Some(aggregator_metadata.blob_data_path.clone());
        state_metadata.da_segment_path = Some(aggregator_metadata.da_segment_path.clone());

        Ok(())
    }
}
