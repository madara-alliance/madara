use crate::core::config::Config;
use crate::error::job::state_update::StateUpdateError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::constant::{ON_CHAIN_DATA_FILE_NAME, PROOF_FILE_NAME, PROOF_PART2_FILE_NAME};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, StateUpdateMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::OnChainData;
use crate::worker::utils::{fetch_blob_data_for_block, fetch_program_output_for_block, fetch_snos_for_block};
use async_trait::async_trait;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use orchestrator_settlement_client_interface::SettlementVerificationStatus;
use orchestrator_utils::collections::{has_dup, is_sorted};
use starknet_core::types::Felt;
use starknet_os::io::output::StarknetOsOutput;
use std::sync::Arc;
use swiftness_proof_parser::{parse, StarkProof};

pub struct StateUpdateJobHandler;
#[async_trait]
impl JobHandlerTrait for StateUpdateJobHandler {
    #[tracing::instrument(fields(category = "state_update"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(
            log_type = "starting",
            category = "state_update",
            function_type = "create_job",
            block_no = %internal_id,
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
        let job_item = JobItem::create(internal_id.clone(), JobType::StateTransition, JobStatus::Created, metadata);

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

        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into()?;

        self.validate_block_numbers(config.clone(), &state_metadata.blocks_to_settle).await?;

        tracing::debug!(job_id = %job.internal_id, blocks = ?state_metadata.blocks_to_settle, "Validated block numbers");

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
        let mut state_metadata: StateUpdateMetadata = job.metadata.specific.clone().try_into()?;
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

        let last_settled_block_number =
            settlement_client.get_last_settled_block().await.map_err(|e| JobError::Other(OtherError(e)))?;

        match last_settled_block_number {
            Some(block_num) => {
                let block_status = if block_num == *expected_last_block_number {
                    tracing::info!(log_type = "completed", category = "state_update", function_type = "verify_job", job_id = %job.id,  block_no = %internal_id, last_settled_block = %block_num, "Last settled block verified.");
                    SettlementVerificationStatus::Verified
                } else {
                    tracing::warn!(log_type = "failed/rejected", category = "state_update", function_type = "verify_job", job_id = %job.id,  block_no = %internal_id, expected = %expected_last_block_number, actual = %block_num, "Last settled block mismatch.");
                    SettlementVerificationStatus::Rejected(format!(
                        "Last settle bock expected was {} but found {}",
                        expected_last_block_number, block_num
                    ))
                };
                Ok(block_status.into())
            }
            None => {
                panic!("How do we still have special_address_ after settling")
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
            let proof_key = format!("{block_no}/{PROOF_FILE_NAME}");
            tracing::debug!(%proof_key, "Fetching snos proof file");

            let proof_file = config.storage().get_data(&proof_key).await?;

            let snos_proof = String::from_utf8(proof_file.to_vec()).map_err(|e| {
                tracing::error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let parsed_snos_proof: StarkProof = parse(snos_proof.clone()).map_err(|e| {
                tracing::error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let proof_key = format!("{block_no}/{PROOF_PART2_FILE_NAME}");
            tracing::debug!(%proof_key, "Fetching 2nd proof file");

            let proof_file = config.storage().get_data(&proof_key).await?;

            let second_proof = String::from_utf8(proof_file.to_vec()).map_err(|e| {
                tracing::error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let parsed_bridge_proof: StarkProof = parse(second_proof.clone()).map_err(|e| {
                tracing::error!(error = %e, "Failed to parse proof file as UTF-8");
                JobError::Other(OtherError(eyre!("{}", e)))
            })?;

            let snos_output = vec_felt_to_vec_bytes32(calculate_output(parsed_snos_proof));
            let program_output = vec_felt_to_vec_bytes32(calculate_output(parsed_bridge_proof));

            // let program_output = self.fetch_program_output_for_block(block_no, config.clone()).await;
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
