use std::sync::Arc;

use crate::core::config::Config;
use crate::error::job::state_update::StateUpdateError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, StateUpdateMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::helpers::JobProcessingState;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::{fetch_blob_data_for_block, fetch_program_output_for_block, fetch_snos_for_block};
use async_trait::async_trait;
use cairo_vm::Felt252;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use orchestrator_settlement_client_interface::SettlementVerificationStatus;
use orchestrator_utils::collections::{has_dup, is_sorted};
use starknet_os::io::output::StarknetOsOutput;
use uuid::Uuid;
use crate::types::batch::BatchStatus;

pub struct StateUpdateJobHandler;
#[async_trait]
impl JobHandlerTrait for StateUpdateJobHandler {
    #[tracing::instrument(fields(category = "state_update"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "state_update",
            function_type = "create_job",
            batch_no = %internal_id,
            "State update job creation started."
        );

        // Extract state transition metadata
        let state_metadata: StateUpdateMetadata = metadata.specific.clone().try_into()?;

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
            batch_no = %internal_id,
            blocks_to_settle = ?state_metadata.batches_to_settle,
            "State update job created."
        );

        Ok(job_item)
    }

    /// This method is for processing state transition jobs
    /// 1. Get the batches to do state transition for
    /// 2. Filter these if state transition jobs failed for some batches before
    /// 3. Fetch snos output, program output and blob data from storage
    /// 4. Perform state transition for all batches one by one
    #[tracing::instrument(fields(category = "state_update"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "state_update",
            function_type = "process_job",
            job_id = %job.id,
            batch_no = %internal_id,
            "State update job processing started."
        );

        // Get the state transition metadata
        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into()?;

        // TODO: update this parameters being passed to this function to pass the blocks to settle by combining the blocks of all batches into a single vector
        // Validate if the batch numbers are correct
        self.validate_block_numbers(config.clone(), &state_metadata.batches_to_settle).await?;

        // Filter block numbers if there was a previous failure
        let last_failed_batch = state_metadata.last_failed_batch_no.unwrap_or(1);  // The lowest possible batch number is 1
        let filtered_indices: Vec<usize> = state_metadata
            .batches_to_settle
            .iter()
            .enumerate()
            .filter(|(_, &block)| block >= last_failed_batch)
            .map(|(i, _)| i)
            .collect();

        let snos_output_paths = state_metadata.snos_output_paths.clone();
        let program_output_paths = state_metadata.program_output_paths.clone();
        let blob_data_paths = state_metadata.blob_data_paths.clone();

        let mut nonce = config.settlement_client().get_nonce().await.map_err(|e| JobError::Other(OtherError(e)))?;

        let mut sent_tx_hashes: Vec<String> = Vec::with_capacity(filtered_indices.len());

        // Loop over the indices to process
        for &i in &filtered_indices {
            let batch_no = state_metadata.batches_to_settle[i];
            tracing::debug!(job_id = %job.internal_id, batch_no = %batch_no, "Processing batch");

            // Get the snos output, program output and blob data for the batch
            let snos_output = fetch_snos_for_block(internal_id.clone(), i, config.clone(), &snos_output_paths).await?;
            let program_output = fetch_program_output_for_block(i, config.clone(), &program_output_paths).await?;
            let blob_data = fetch_blob_data_for_block(i, config.clone(), &blob_data_paths).await?;

            // TODO: Check if this will work
            // Perform state update
            let txn_hash = match self
                .update_state_for_block(config.clone(), batch_no, snos_output, nonce, program_output, blob_data)
                .await
            {
                Ok(hash) => hash,
                Err(e) => {
                    tracing::error!(job_id = %job.internal_id, batch_no = %batch_no, error = %e, "Error updating state for block");
                    state_metadata.last_failed_batch_no = Some(batch_no);
                    state_metadata.tx_hashes = sent_tx_hashes.clone();
                    job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());

                    return Err(JobError::Other(OtherError(eyre!(
                        "Block #{batch_no} - Error occurred during the state update: {e}"
                    ))));
                }
            };

            sent_tx_hashes.push(txn_hash);
            state_metadata.tx_hashes = sent_tx_hashes.clone();
            job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());
            nonce += 1;
            config.database().update_batch_status_by_index(batch_no, BatchStatus::Completed).await?;
        }

        let val = state_metadata.batches_to_settle.last().ok_or_else(|| StateUpdateError::LastNumberReturnedError)?;

        tracing::info!(
            log_type = "completed",
            category = "state_update",
            function_type = "process_job",
            job_id = %job.id,
            batch_no = %internal_id,
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
            batch_no = %internal_id,
            "State update job verification started."
        );

        // Get state update metadata
        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into()?;
        // Get transaction hashes
        let tx_hashes = state_metadata.tx_hashes;

        let batch_numbers = state_metadata.batches_to_settle;
        tracing::debug!(job_id = %job.internal_id, "Retrieved batch numbers from metadata");
        let settlement_client = config.settlement_client();

        for (tx_hash, batch_no) in tx_hashes.iter().zip(batch_numbers.iter()) {
            tracing::trace!(
                job_id = %job.internal_id,
                tx_hash = %tx_hash,
                batch_no = %batch_no,
                "Verifying transaction inclusion"
            );

            let tx_inclusion_status =
                settlement_client.verify_tx_inclusion(tx_hash).await.map_err(|e| JobError::Other(OtherError(e)))?;

            match tx_inclusion_status {
                SettlementVerificationStatus::Rejected(_) => {
                    tracing::warn!(
                        job_id = %job.internal_id,
                        tx_hash = %tx_hash,
                        batch_no = %batch_no,
                        "Transaction rejected"
                    );
                    state_metadata.last_failed_batch_no = Some(*batch_no);
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
                                batch_no = %batch_no,
                                "Transaction rejected after finality"
                            );
                            state_metadata.last_failed_batch_no = Some(*batch_no);
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
        let expected_last_block_number = batch_numbers.last().ok_or_else(|| StateUpdateError::EmptyBlockNumberList)?;

        let last_settled_block_number =
            settlement_client.get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;

        match last_settled_block_number {
            Some(block_num) => {
                let block_status = if block_num == *expected_last_block_number {
                    tracing::info!(
                        log_type = "completed",
                        category = "state_update",
                        function_type = "verify_job",
                        job_id = %job.id,
                        batch_no = %internal_id,
                        last_settled_block = %block_num,
                        "Last settled block verified."
                    );
                    SettlementVerificationStatus::Verified
                } else {
                    tracing::warn!(
                        log_type = "failed/rejected",
                        category = "state_update",
                        function_type = "verify_job",
                        job_id = %job.id,
                        batch_no = %internal_id,
                        expected = %expected_last_block_number,
                        actual = %block_num,
                        "Last settled block mismatch."
                    );
                    SettlementVerificationStatus::Rejected(format!(
                        "Last settle bock expected was {} but found {}",
                        expected_last_block_number, block_num
                    ))
                };
                Ok(block_status.into())
            }
            None => {
                panic!("Incorrect state after settling blocks")
            }
        }
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

    fn job_processing_lock(&self, _config: Arc<Config>) -> Option<Arc<JobProcessingState>> {
        None
    }
}

impl StateUpdateJobHandler {
    /// Validate that the list of block numbers to process is valid.
    async fn validate_block_numbers(&self, config: Arc<Config>, block_numbers: &[u64]) -> Result<(), JobError> {
        // if any block is settled then previous block number should be just before that
        // if no block is settled (confirmed by special number) then the block to settle should be 0

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
        let last_settled_block: Option<u64> =
            config.settlement_client().get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;
        let expected_first_block = last_settled_block.map_or(0, |num| num + 1);

        if block_numbers[0] != expected_first_block {
            return Err(StateUpdateError::GapBetweenFirstAndLastBlock.into());
        }

        Ok(())
    }

    /// Update the state for the corresponding block using the settlement layer.
    #[allow(clippy::too_many_arguments)]
    async fn update_state_for_block(
        &self,
        config: Arc<Config>,
        block_no: u64,
        snos_output: StarknetOsOutput,
        nonce: u64,
        program_output: Vec<[u8; 32]>,
        blob_data: Vec<Vec<u8>>,
    ) -> Result<String, JobError> {
        let settlement_client = config.settlement_client();
        let last_tx_hash_executed = if snos_output.use_kzg_da == Felt252::ZERO {
            unimplemented!("update_state_for_block not implemented as of now for calldata DA.")
        } else if snos_output.use_kzg_da == Felt252::ONE {
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
