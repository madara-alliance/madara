use crate::core::config::Config;
use crate::error::job::state_update::StateUpdateError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::constant::{ON_CHAIN_DATA_FILE_NAME, PROOF_FILE_NAME, PROOF_PART2_FILE_NAME};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::OnChainData;
use crate::worker::utils::{
    fetch_blob_data_for_batch, fetch_blob_data_for_block, fetch_program_output_for_block, fetch_snos_for_block,
};
use async_trait::async_trait;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use orchestrator_settlement_client_interface::SettlementVerificationStatus;
use orchestrator_utils::collections::{has_dup, is_sorted};
use orchestrator_utils::layer::Layer;
use starknet_core::types::Felt;
use starknet_os::io::output::StarknetOsOutput;
use std::sync::Arc;
use swiftness_proof_parser::{parse, StarkProof};
use tracing::{debug, error, info, trace, warn};

struct StateUpdateArtifacts {
    snos_output: Option<StarknetOsOutput>,
    program_output: Vec<[u8; 32]>,
    blob_data: Vec<Vec<u8>>,
}

pub struct StateUpdateJobHandler;
#[async_trait]
impl JobHandlerTrait for StateUpdateJobHandler {
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        info!(log_type = "starting", "State update job creation started.");

        // Extract state transition metadata
        let state_metadata: StateUpdateMetadata = metadata.specific.clone().try_into()?;

        // Validate required paths
        if state_metadata.snos_output_paths.is_empty()
            || state_metadata.program_output_paths.is_empty()
            || state_metadata.blob_data_paths.is_empty()
        {
            error!("Missing required paths in metadata");
            return Err(JobError::Other(OtherError(eyre!("Missing required paths in metadata"))));
        }
        let job_item = JobItem::create(internal_id.clone(), JobType::StateTransition, JobStatus::Created, metadata);

        info!(
            log_type = "completed",
            context = ?state_metadata.context,
            "State update job created."
        );

        Ok(job_item)
    }

    /// This method is for processing state transition jobs.
    /// It handles both L2 and L3 state updates.
    /// For L2, it does the state update using blobs.
    /// For L3, it does the state update using call data.
    /// 1. Get the blocks/batches to do state transition for
    /// 2. Filter these if state transition jobs failed for some blocks/batches before
    /// 3. Fetch snos output, program output and blob data from storage
    /// 4. Perform state transition for all blocks/batches one by one
    ///
    /// NOTE: Right now, if any job fails, the whole orchestrator halts.
    /// So, the retry logic added here (for restarting the state update from the block which failed
    /// last time) will not actually work.
    ///
    /// TODO: Update the code in the future releases to fix this.
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        info!(log_type = "starting", "State update job processing started.");

        // Get the state transition metadata
        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into()?;

        let (block_or_batch_to_settle, last_failed_block_or_batch) = match state_metadata.context.clone() {
            SettlementContext::Block(data) => {
                self.validate_block_numbers(config.clone(), &data.to_settle).await?;
                debug!(blocks = ?data.to_settle, "Validated block numbers");
                if !data.to_settle.is_empty() {
                    tracing::Span::current().record("block_start", data.to_settle[0]);
                    tracing::Span::current().record("block_end", data.to_settle[data.to_settle.len() - 1]);
                }
                (data.to_settle, data.last_failed.unwrap_or(0))
            }
            SettlementContext::Batch(data) => {
                if !data.to_settle.is_empty() {
                    tracing::Span::current().record("batch_start", data.to_settle[0]);
                    tracing::Span::current().record("batch_end", data.to_settle[data.to_settle.len() - 1]);
                }
                (data.to_settle, data.last_failed.unwrap_or(1)) // The lowest possible batch number is 1
            }
        };

        // Filter block numbers if there was a previous failure
        let filtered_indices: Vec<usize> = block_or_batch_to_settle
            .iter()
            .enumerate()
            .filter(|(_, &num)| num >= last_failed_block_or_batch)
            .map(|(i, _)| i)
            .collect();

        let snos_output_paths = state_metadata.snos_output_paths.clone();
        let program_output_paths = state_metadata.program_output_paths.clone();
        let blob_data_paths = state_metadata.blob_data_paths.clone();

        let mut nonce = config.settlement_client().get_nonce().await.map_err(|e| JobError::Other(OtherError(e)))?;

        let mut sent_tx_hashes: Vec<String> = Vec::with_capacity(filtered_indices.len());

        // Loop over the indices to process
        for &i in &filtered_indices {
            let to_settle_num = block_or_batch_to_settle[i];
            debug!(num = %to_settle_num, "Processing block/batch");

            // Get the artifacts for the block/batch
            let snos_output =
                match fetch_snos_for_block(internal_id.clone(), i, config.clone(), &snos_output_paths).await {
                    Ok(snos_output) => Some(snos_output),
                    Err(err) => {
                        debug!("failed to fetch snos output, proceeding without it: {}", err);
                        None
                    }
                };
            let program_output = fetch_program_output_for_block(i, config.clone(), &program_output_paths).await?;
            let blob_data = match config.layer() {
                Layer::L2 => fetch_blob_data_for_batch(i, config.clone(), &blob_data_paths).await?,
                Layer::L3 => fetch_blob_data_for_block(i, config.clone(), &blob_data_paths).await?,
            };

            let txn_hash = match self
                .update_state(
                    config.clone(),
                    to_settle_num,
                    nonce,
                    StateUpdateArtifacts { snos_output, program_output, blob_data },
                )
                .await
            {
                Ok(hash) => hash,
                Err(e) => {
                    error!(num = %to_settle_num, error = %e, "Error updating state for block/batch");
                    state_metadata.context = self.update_last_failed(state_metadata.context.clone(), to_settle_num);
                    state_metadata.tx_hashes = sent_tx_hashes.clone();
                    job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());

                    return Err(JobError::Other(OtherError(eyre!("Error occurred during the state update: {e}"))));
                }
            };

            debug!(num = %to_settle_num, tx_hash = %txn_hash, "Validating transaction receipt");

            config.settlement_client().wait_for_tx_finality(&txn_hash).await.map_err(|e| {
                error!(num = %to_settle_num, tx_hash = %txn_hash, error = %e, "Error waiting for transaction finality");
                JobError::Other(OtherError(e))
            })?;

            sent_tx_hashes.push(txn_hash);
            state_metadata.tx_hashes = sent_tx_hashes.clone();
            job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());
            nonce += 1;
        }

        let val = block_or_batch_to_settle.last().ok_or_else(|| StateUpdateError::LastNumberReturnedError)?;

        info!(
            log_type = "completed",
            last_settled_block = %val,
            "State update job processed successfully."
        );

        Ok(val.to_string())
    }

    /// Returns the status of the passed job.
    /// Status will be verified if:
    /// 1. The last settlement tx hash is successful,
    /// 2. The expected last settled block from our configuration is indeed the one found in the
    ///    provider.
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        info!(log_type = "starting", "State update job verification started.");

        // Get state update metadata
        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into()?;
        // Get transaction hashes
        let tx_hashes = state_metadata.tx_hashes.clone();

        let nums_settled = match state_metadata.context.clone() {
            SettlementContext::Block(data) => data.to_settle,
            SettlementContext::Batch(data) => data.to_settle,
        };

        debug!("Retrieved block numbers from metadata");
        let settlement_client = config.settlement_client();

        for (tx_hash, num_settled) in tx_hashes.iter().zip(nums_settled.iter()) {
            trace!(
                tx_hash = %tx_hash,
                num = %num_settled,
                "Verifying transaction inclusion"
            );

            let tx_inclusion_status =
                settlement_client.verify_tx_inclusion(tx_hash).await.map_err(|e| JobError::Other(OtherError(e)))?;

            match tx_inclusion_status {
                SettlementVerificationStatus::Rejected(_) => {
                    warn!(
                        tx_hash = %tx_hash,
                        num = %num_settled,
                        "Transaction rejected"
                    );
                    state_metadata.context = self.update_last_failed(state_metadata.context.clone(), *num_settled);
                    job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());
                    return Ok(tx_inclusion_status.into());
                }
                // If the tx is still pending, we wait for it to be finalized and check again the status.
                SettlementVerificationStatus::Pending => {
                    debug!(
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
                            warn!(
                                tx_hash = %tx_hash,
                                num = %num_settled,
                                "Transaction rejected after finality"
                            );
                            state_metadata.context =
                                self.update_last_failed(state_metadata.context.clone(), *num_settled);
                            job.metadata.specific = JobSpecificMetadata::StateUpdate(state_metadata.clone());
                            return Ok(new_status.into());
                        }
                        SettlementVerificationStatus::Pending => {
                            error!(
                                tx_hash = %tx_hash,
                                "Transaction still pending after finality check"
                            );
                            Err(StateUpdateError::TxnShouldNotBePending { tx_hash: tx_hash.to_string() })?
                        }
                        SettlementVerificationStatus::Verified => {
                            debug!(
                                tx_hash = %tx_hash,
                                "Transaction verified after finality"
                            );
                        }
                    }
                }
                SettlementVerificationStatus::Verified => {
                    debug!(
                        tx_hash = %tx_hash,
                        "Transaction verified"
                    );
                }
            }
        }

        // verify that the last settled block is indeed the one we expect to be
        let last_settled = nums_settled.last().ok_or_else(|| StateUpdateError::EmptyBlockNumberList)?;
        let expected_last_block_number = match config.layer() {
            Layer::L2 => {
                // Get the batch details for the last-settled batch
                let batch = config.database().get_batches_by_indexes(vec![*last_settled]).await?;
                if batch.is_empty() {
                    Err(JobError::Other(OtherError(eyre!("Failed to fetch batch {} from database", last_settled))))
                } else {
                    // Return the end block of the last batch
                    Ok(batch[0].end_block)
                }
            }
            Layer::L3 => Ok(*last_settled),
        }?;

        let last_settled_block_number =
            settlement_client.get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;

        match last_settled_block_number {
            Some(block_num) => {
                let block_status = if block_num == expected_last_block_number {
                    info!(log_type = "completed", last_settled_block = %block_num, "Last settled block verified.");
                    SettlementVerificationStatus::Verified
                } else {
                    warn!(log_type = "failed/rejected", expected = %expected_last_block_number, actual = %block_num, "Last settled block mismatch.");
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
}

impl StateUpdateJobHandler {
    fn update_last_failed(&self, settlement_context: SettlementContext, failed: u64) -> SettlementContext {
        match settlement_context {
            SettlementContext::Block(data) => {
                SettlementContext::Block(SettlementContextData { to_settle: data.to_settle, last_failed: Some(failed) })
            }
            SettlementContext::Batch(data) => {
                SettlementContext::Batch(SettlementContextData { to_settle: data.to_settle, last_failed: Some(failed) })
            }
        }
    }

    /// Validate that the list of block numbers to process is valid.
    async fn validate_block_numbers(&self, config: Arc<Config>, block_numbers: &[u64]) -> Result<(), JobError> {
        // if any block is settled then previous block number should be just before that
        // if no block is settled (confirmed by special number), then the block to settle should be 0

        if block_numbers.is_empty() {
            Err(StateUpdateError::BlockNumberNotFound)?;
        }
        if has_dup(block_numbers) {
            Err(StateUpdateError::DuplicateBlockNumbers)?;
        }
        if !is_sorted(block_numbers) {
            Err(StateUpdateError::UnsortedBlockNumbers)?;
        }

        // Check for any gap between the last settled block and the first block to settle
        let last_settled_block =
            config.settlement_client().get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;

        let expected_first_block = last_settled_block.map_or(0, |block| block + 1);
        if block_numbers[0] != expected_first_block {
            Err(StateUpdateError::GapBetweenFirstAndLastBlock)?;
        }

        Ok(())
    }

    /// Retrieves the OnChain data for the corresponding block.
    async fn fetch_onchain_data_for_block(&self, block_number: u64, config: Arc<Config>) -> OnChainData {
        let storage_client = config.storage();
        let key = block_number.to_string() + "/" + ON_CHAIN_DATA_FILE_NAME;
        let onchain_data_bytes = storage_client.get_data(&key).await.expect("Unable to fetch onchain data for block");
        serde_json::from_slice(onchain_data_bytes.iter().as_slice())
            .expect("Unable to convert the data into onchain data")
    }

    async fn update_state(
        &self,
        config: Arc<Config>,
        to_settle_num: u64,
        nonce: u64,
        artifacts: StateUpdateArtifacts,
    ) -> Result<String, JobError> {
        match config.layer() {
            Layer::L2 => self.update_state_for_batch(config, nonce, artifacts).await,
            Layer::L3 => self.update_state_for_block(config, to_settle_num, artifacts).await,
        }
    }

    async fn update_state_for_batch(
        &self,
        config: Arc<Config>,
        nonce: u64,
        artifacts: StateUpdateArtifacts,
    ) -> Result<String, JobError> {
        let settlement_client = config.settlement_client();
        settlement_client
            .update_state_with_blobs(artifacts.program_output, artifacts.blob_data, nonce)
            .await
            .map_err(|e| JobError::Other(OtherError(e)))
    }

    /// Update the state for the corresponding block using the settlement layer.
    async fn update_state_for_block(
        &self,
        config: Arc<Config>,
        block_no: u64,
        artifacts: StateUpdateArtifacts,
    ) -> Result<String, JobError> {
        let settlement_client = config.settlement_client();

        // Trying to get snos output
        let snos_output = match artifacts.snos_output {
            Some(snos_output) => snos_output,
            None => {
                error!("SnosOutput not found during state update for block. Cannot proceed without it!");
                return Err(JobError::Other(OtherError(eyre!(
                    "SnosOutput not found during state update for block. Cannot proceed without it!"
                ))));
            }
        };

        // Check for the correctness of the use_kzg_da flag in snos_output
        let last_tx_hash_executed = if snos_output.use_kzg_da == Felt252::ZERO {
            let proof_key = format!("{block_no}/{PROOF_FILE_NAME}");
            debug!(%proof_key, "Fetching snos proof file");

            let proof_file = config.storage().get_data(&proof_key).await?;

            let snos_proof = String::from_utf8(proof_file.to_vec()).map_err(|e| {
                error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let parsed_snos_proof: StarkProof = parse(snos_proof.clone()).map_err(|e| {
                error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let proof_key = format!("{block_no}/{PROOF_PART2_FILE_NAME}");
            debug!(%proof_key, "Fetching 2nd proof file");

            let proof_file = config.storage().get_data(&proof_key).await?;

            let second_proof = String::from_utf8(proof_file.to_vec()).map_err(|e| {
                error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let parsed_bridge_proof: StarkProof = parse(second_proof.clone()).map_err(|e| {
                error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let snos_output = vec_felt_to_vec_bytes32(calculate_output(parsed_snos_proof));
            let program_output = vec_felt_to_vec_bytes32(calculate_output(parsed_bridge_proof));

            let onchain_data = self.fetch_onchain_data_for_block(block_no, config.clone()).await;
            settlement_client
                .update_state_calldata(
                    snos_output,
                    program_output,
                    onchain_data.on_chain_data_hash.0,
                    usize_to_bytes(onchain_data.on_chain_data_size),
                )
                .await
                .map_err(|e| JobError::Other(OtherError(e)))?
        } else {
            Err(StateUpdateError::UseKZGDaError { block_no })?
        };

        Ok(last_tx_hash_executed)
    }
}

pub fn calculate_output(proof: StarkProof) -> Vec<Felt> {
    let output_segment = proof.public_input.segments[2].clone();
    let output_len = output_segment.stop_ptr - output_segment.begin_addr;
    let start = proof.public_input.main_page.len() - output_len as usize;
    let end = proof.public_input.main_page.len();
    let program_output =
        proof.public_input.main_page[start..end].iter().map(|cell| cell.value.clone()).collect::<Vec<_>>();
    let mut felts = vec![];
    for elem in &program_output {
        felts.push(Felt::from_dec_str(&elem.to_string()).unwrap());
    }
    felts
}

pub fn vec_felt_to_vec_bytes32(felts: Vec<Felt>) -> Vec<[u8; 32]> {
    felts
        .into_iter()
        .map(|felt| {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&felt.to_bytes_be());
            bytes
        })
        .collect()
}

fn usize_to_bytes(n: usize) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&n.to_le_bytes());
    bytes
}
