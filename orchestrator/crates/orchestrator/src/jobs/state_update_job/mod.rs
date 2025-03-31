pub mod utils;

use std::sync::Arc;

use async_trait::async_trait;
use cairo_vm::Felt252;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use orchestrator_settlement_client_interface::SettlementVerificationStatus;
use orchestrator_utils::collections::{has_dup, is_sorted};
use starknet_os::io::output::StarknetOsOutput;
use thiserror::Error;
use uuid::Uuid;

use super::{JobError, OtherError};
use crate::config::Config;
use crate::utils::helpers;
use crate::jobs::metadata::{JobMetadata, JobSpecificMetadata, StateUpdateMetadata};
use crate::jobs::state_update_job::utils::{
    fetch_blob_data_for_block, fetch_program_output_for_block, fetch_snos_for_block,
};
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

#[derive(Error, Debug, PartialEq)]
pub enum StateUpdateError {
    #[error("Block numbers list should not be empty.")]
    EmptyBlockNumberList,

    #[error("Could not find current attempt number.")]
    AttemptNumberNotFound,

    #[error("last_failed_block should be a positive number")]
    LastFailedBlockNonPositive,

    #[error("Block numbers to settle must be specified (state update job #{internal_id:?})")]
    UnspecifiedBlockNumber { internal_id: String },

    #[error("Could not find tx hashes metadata for the current attempt")]
    TxnHashMetadataNotFound,

    #[error("Tx {tx_hash:?} should not be pending.")]
    TxnShouldNotBePending { tx_hash: String },

    #[error(
        "Last number in block_numbers array returned as None. Possible Error : Delay in job processing or Failed job \
         execution."
    )]
    LastNumberReturnedError,

    #[error("No block numbers found.")]
    BlockNumberNotFound,

    #[error("Duplicated block numbers.")]
    DuplicateBlockNumbers,

    #[error("Block numbers aren't sorted in increasing order.")]
    UnsortedBlockNumbers,

    #[error("Gap detected between the first block to settle and the last one settled.")]
    GapBetweenFirstAndLastBlock,

    #[error("Block #{block_no:?} - SNOS error, [use_kzg_da] should be either 0 or 1.")]
    UseKZGDaError { block_no: u64 },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

pub struct StateUpdateJob;
#[async_trait]
impl Job for StateUpdateJob {
    #[tracing::instrument(fields(category = "state_update"), skip(self, _config, metadata), ret, err)]
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: JobMetadata,
    ) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "state_update",
            function_type = "create_job",
            block_no = %internal_id,
            "State update job creation started."
        );

        // Extract state transition metadata
        let state_metadata: StateUpdateMetadata = metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = %internal_id, error = %e, "Invalid metadata type for state update job");
            JobError::Other(OtherError(e))
        })?;

        // Validate required paths
        if state_metadata.snos_output_paths.is_empty()
            || state_metadata.program_output_paths.is_empty()
            || state_metadata.blob_data_paths.is_empty()
        {
            tracing::error!(job_id = %internal_id, "Missing required paths in metadata");
            return Err(JobError::Other(OtherError(eyre!("Missing required paths in metadata"))));
        }

        // Create job with initialized metadata
        let job_item = JobItem {
            id: Uuid::new_v4(),
            internal_id: internal_id.clone(),
            job_type: JobType::StateTransition,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata: metadata.clone(),
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        };

        tracing::info!(
            log_type = "completed",
            category = "state_update",
            function_type = "create_job",
            block_no = %internal_id,
            blocks_to_settle = ?state_metadata.blocks_to_settle,
            "State update job created."
        );

        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "state_update"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "state_update",
            function_type = "process_job",
            job_id = %job.id,
            block_no = %internal_id,
            "State update job processing started."
        );

        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = %internal_id, error = %e, "Invalid metadata type for state update job");
            JobError::Other(OtherError(e))
        })?;

        self.validate_block_numbers(config.clone(), &state_metadata.blocks_to_settle).await?;

        // Filter block numbers if there was a previous failure
        let last_failed_block = state_metadata.last_failed_block_no.unwrap_or(0);
        let filtered_indices: Vec<usize> = state_metadata
            .blocks_to_settle
            .iter()
            .enumerate()
            .filter(|(_, &block)| block >= last_failed_block)
            .map(|(i, _)| i)
            .collect();

        let snos_output_paths = state_metadata.snos_output_paths.clone();
        let program_output_paths = state_metadata.program_output_paths.clone();
        let blob_data_paths = state_metadata.blob_data_paths.clone();

        let mut nonce = config.settlement_client().get_nonce().await.map_err(|e| JobError::Other(OtherError(e)))?;

        let mut sent_tx_hashes: Vec<String> = Vec::with_capacity(filtered_indices.len());

        for &i in &filtered_indices {
            let block_no = state_metadata.blocks_to_settle[i];
            tracing::debug!(job_id = %job.internal_id, block_no = %block_no, "Processing block");
            let snos = fetch_snos_for_block(internal_id.clone(), i, config.clone(), &snos_output_paths).await?;
            let program_output = fetch_program_output_for_block(i, config.clone(), &program_output_paths).await?;
            let blob_data = fetch_blob_data_for_block(i, config.clone(), &blob_data_paths).await?;
            let txn_hash = match self
                .update_state_for_block(config.clone(), block_no, snos, nonce, program_output, blob_data)
                .await
            {
                Ok(hash) => hash,
                Err(e) => {
                    tracing::error!(job_id = %job.internal_id, block_no = %block_no, error = %e, "Error updating state for block");
                    state_metadata.last_failed_block_no = Some(block_no);
                    state_metadata.tx_hashes = sent_tx_hashes.clone();
                    job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());

                    return Err(JobError::Other(OtherError(eyre!(
                        "Block #{block_no} - Error occurred during the state update: {e}"
                    ))));
                }
            };

            sent_tx_hashes.push(txn_hash);
            state_metadata.tx_hashes = sent_tx_hashes.clone();
            job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());
            nonce += 1;
        }

        let val = state_metadata.blocks_to_settle.last().ok_or_else(|| StateUpdateError::LastNumberReturnedError)?;

        tracing::info!(
            log_type = "completed",
            category = "state_update",
            function_type = "process_job",
            job_id = %job.id,
            block_no = %internal_id,
            last_settled_block = %val,
            "State update job processed successfully."
        );

        Ok(val.to_string())
    }

    /// Returns the status of the passed job.
    /// Status will be verified if:
    /// 1. the last settlement tx hash is successful,
    /// 2. the expected last settled block from our configuration is indeed the one found in the
    ///    provider.
    #[tracing::instrument(fields(category = "state_update"), skip(self, config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "state_update",
            function_type = "verify_job",
            job_id = %job.id,
            block_no = %internal_id,
            "State update job verification started."
        );

        // Get state update metadata
        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = ?job.id, error = ?e, "Invalid metadata type for state update job");
            JobError::Other(OtherError(e))
        })?;
        // Get transaction hashes
        let tx_hashes = state_metadata.tx_hashes;

        let block_numbers = state_metadata.blocks_to_settle;
        tracing::debug!(job_id = %job.internal_id, "Retrieved block numbers from metadata");
        let settlement_client = config.settlement_client();

        for (tx_hash, block_no) in tx_hashes.iter().zip(block_numbers.iter()) {
            tracing::trace!(
                job_id = %job.internal_id,
                tx_hash = %tx_hash,
                block_no = %block_no,
                "Verifying transaction inclusion"
            );

            let tx_inclusion_status =
                settlement_client.verify_tx_inclusion(tx_hash).await.map_err(|e| JobError::Other(OtherError(e)))?;

            match tx_inclusion_status {
                SettlementVerificationStatus::Rejected(_) => {
                    tracing::warn!(
                        job_id = %job.internal_id,
                        tx_hash = %tx_hash,
                        block_no = %block_no,
                        "Transaction rejected"
                    );
                    state_metadata.last_failed_block_no = Some(*block_no);
                    return Ok(tx_inclusion_status.into());
                }
                // If the tx is still pending, we wait for it to be finalized and check again the status.
                SettlementVerificationStatus::Pending => {
                    tracing::debug!(
                        job_id = %job.internal_id,
                        tx_hash = %tx_hash,
                        "Transaction pending, waiting for finality"
                    );
                    settlement_client
                        .wait_for_tx_finality(tx_hash)
                        .await
                        .map_err(|e| JobError::Other(OtherError(e)))?;

                    let new_status = settlement_client
                        .verify_tx_inclusion(tx_hash)
                        .await
                        .map_err(|e| JobError::Other(OtherError(e)))?;

                    match new_status {
                        SettlementVerificationStatus::Rejected(_) => {
                            tracing::warn!(
                                job_id = %job.internal_id,
                                tx_hash = %tx_hash,
                                block_no = %block_no,
                                "Transaction rejected after finality"
                            );
                            state_metadata.last_failed_block_no = Some(*block_no);
                            return Ok(new_status.into());
                        }
                        SettlementVerificationStatus::Pending => {
                            tracing::error!(
                                job_id = %job.internal_id,
                                tx_hash = %tx_hash,
                                "Transaction still pending after finality check"
                            );
                            Err(StateUpdateError::TxnShouldNotBePending { tx_hash: tx_hash.to_string() })?
                        }
                        SettlementVerificationStatus::Verified => {
                            tracing::debug!(
                                job_id = %job.internal_id,
                                tx_hash = %tx_hash,
                                "Transaction verified after finality"
                            );
                        }
                    }
                }
                SettlementVerificationStatus::Verified => {
                    tracing::debug!(
                        job_id = %job.internal_id,
                        tx_hash = %tx_hash,
                        "Transaction verified"
                    );
                }
            }
        }

        // verify that the last settled block is indeed the one we expect to be
        let expected_last_block_number = block_numbers.last().ok_or_else(|| StateUpdateError::EmptyBlockNumberList)?;

        let out_last_block_number =
            settlement_client.get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;

        let block_status = if out_last_block_number == *expected_last_block_number {
            tracing::info!(
                log_type = "completed",
                category = "state_update",
                function_type = "verify_job",
                job_id = %job.id,
                block_no = %internal_id,
                last_settled_block = %out_last_block_number,
                "Last settled block verified."
            );
            SettlementVerificationStatus::Verified
        } else {
            tracing::warn!(
                log_type = "failed/rejected",
                category = "state_update",
                function_type = "verify_job",
                job_id = %job.id,
                block_no = %internal_id,
                expected = %expected_last_block_number,
                actual = %out_last_block_number,
                "Last settled block mismatch."
            );
            SettlementVerificationStatus::Rejected(format!(
                "Last settle bock expected was {} but found {}",
                expected_last_block_number, out_last_block_number
            ))
        };
        Ok(block_status.into())
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        10
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        60
    }

    fn job_processing_lock(
        &self,
        _config: Arc<Config>,
    ) -> std::option::Option<std::sync::Arc<helpers::JobProcessingState>> {
        None
    }
}

impl StateUpdateJob {
    /// Validate that the list of block numbers to process is valid.
    async fn validate_block_numbers(&self, config: Arc<Config>, block_numbers: &[u64]) -> Result<(), JobError> {
        if block_numbers.is_empty() {
            Err(StateUpdateError::BlockNumberNotFound)?;
        }
        if has_dup(block_numbers) {
            Err(StateUpdateError::DuplicateBlockNumbers)?;
        }
        if !is_sorted(block_numbers) {
            Err(StateUpdateError::UnsortedBlockNumbers)?;
        }
        // Check for gap between the last settled block and the first block to settle
        let last_settled_block: u64 =
            config.settlement_client().get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;
        if last_settled_block + 1 != block_numbers[0] {
            Err(StateUpdateError::GapBetweenFirstAndLastBlock)?;
        }
        Ok(())
    }

    /// Update the state for the corresponding block using the settlement layer.
    #[allow(clippy::too_many_arguments)]
    async fn update_state_for_block(
        &self,
        config: Arc<Config>,
        block_no: u64,
        snos: StarknetOsOutput,
        nonce: u64,
        program_output: Vec<[u8; 32]>,
        blob_data: Vec<Vec<u8>>,
    ) -> Result<String, JobError> {
        let settlement_client = config.settlement_client();
        let last_tx_hash_executed = if snos.use_kzg_da == Felt252::ZERO {
            unimplemented!("update_state_for_block not implemented as of now for calldata DA.")
        } else if snos.use_kzg_da == Felt252::ONE {
            settlement_client
                .update_state_with_blobs(program_output, blob_data, nonce)
                .await
                .map_err(|e| JobError::Other(OtherError(e)))?
        } else {
            Err(StateUpdateError::UseKZGDaError { block_no })?
        };
        Ok(last_tx_hash_executed)
    }
}
