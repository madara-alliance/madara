use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use lazy_static::lazy_static;
use snos::io::output::StarknetOsOutput;
use starknet::providers::Provider;
use starknet_core::types::{BlockId, MaybePendingStateUpdate};
use uuid::Uuid;

use settlement_client_interface::SettlementVerificationStatus;
use utils::collections::{has_dup, is_sorted};

use super::constants::{
    JOB_METADATA_STATE_UPDATE_ATTEMPT_PREFIX, JOB_METADATA_STATE_UPDATE_LAST_FAILED_BLOCK_NO,
    JOB_PROCESS_ATTEMPT_METADATA_KEY,
};

use crate::config::Config;
use crate::jobs::constants::{
    JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY, JOB_METADATA_STATE_UPDATE_FETCH_FROM_TESTS,
};
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

// TODO: remove when data is correctly stored in DB/S3
lazy_static! {
    pub static ref CURRENT_PATH: PathBuf = std::env::current_dir().unwrap();
}

pub struct StateUpdateJob;
#[async_trait]
impl Job for StateUpdateJob {
    async fn create_job(
        &self,
        _config: &Config,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem> {
        Ok(JobItem {
            id: Uuid::new_v4(),
            internal_id,
            job_type: JobType::StateTransition,
            status: JobStatus::Created,
            external_id: String::new().into(),
            // metadata must contain the blocks for which state update will be performed
            // we don't do one job per state update as that makes nonce management complicated
            metadata,
            version: 0,
        })
    }

    async fn process_job(&self, config: &Config, job: &mut JobItem) -> Result<String> {
        let attempt_no =
            job.metadata.get(JOB_PROCESS_ATTEMPT_METADATA_KEY).expect("Could not find current attempt number.").clone();

        // TODO: remove when SNOS is correctly stored in DB/S3
        // Test metadata to fetch the snos output from the test folder, default to False
        let fetch_from_tests =
            job.metadata.get(JOB_METADATA_STATE_UPDATE_FETCH_FROM_TESTS).map_or(false, |value| value == "TRUE");

        // Read the metadata to get the blocks for which state update will be performed.
        // We assume that blocks nbrs are formatted as follow: "2,3,4,5,6".
        let mut block_numbers = self.get_block_numbers_from_metadata(job)?;
        self.validate_block_numbers(config, &block_numbers).await?;

        // If we had a block state update failing last run, we recover from this block
        if let Some(last_failed_block) = job.metadata.get(JOB_METADATA_STATE_UPDATE_LAST_FAILED_BLOCK_NO) {
            let last_failed_block: u64 =
                last_failed_block.parse().expect("last_failed_block should be a positive number");
            block_numbers = block_numbers.into_iter().filter(|&block| block >= last_failed_block).collect::<Vec<u64>>();
        }

        let mut sent_tx_hashes: Vec<String> = Vec::with_capacity(block_numbers.len());
        for block_no in block_numbers.iter() {
            let snos = self.fetch_snos_for_block(*block_no, Some(fetch_from_tests)).await;
            let tx_hash =
                self.update_state_for_block(config, *block_no, snos, Some(fetch_from_tests)).await.map_err(|e| {
                    job.metadata.insert(JOB_METADATA_STATE_UPDATE_LAST_FAILED_BLOCK_NO.into(), block_no.to_string());
                    self.insert_attempts_into_metadata(job, &attempt_no, &sent_tx_hashes);
                    eyre!("Block #{block_no} - Error occured during the state update: {e}")
                })?;
            sent_tx_hashes.push(tx_hash);
        }

        self.insert_attempts_into_metadata(job, &attempt_no, &sent_tx_hashes);

        // external_id returned corresponds to the last block number settled
        Ok(block_numbers.last().unwrap().to_string())
    }

    /// Returns the status of the passed job.
    /// Status will be verified if:
    /// 1. the last settlement tx hash is successful,
    /// 2. the expected last settled block from our configuration is indeed the one found in the provider.
    async fn verify_job(&self, config: &Config, job: &mut JobItem) -> Result<JobVerificationStatus> {
        let attempt_no =
            job.metadata.get(JOB_PROCESS_ATTEMPT_METADATA_KEY).expect("Could not find current attempt number.").clone();
        let metadata_tx_hashes = job
            .metadata
            .get(&format!("{}{}", JOB_METADATA_STATE_UPDATE_ATTEMPT_PREFIX, attempt_no))
            .expect("Could not find tx hashes metadata for the current attempt")
            .clone()
            .replace(' ', "");

        let tx_hashes: Vec<&str> = metadata_tx_hashes.split(',').collect();
        let block_numbers = self.get_block_numbers_from_metadata(job)?;
        let settlement_client = config.settlement_client();

        for (tx_hash, block_no) in tx_hashes.iter().zip(block_numbers.iter()) {
            let tx_inclusion_status = settlement_client.verify_tx_inclusion(tx_hash).await?;
            match tx_inclusion_status {
                SettlementVerificationStatus::Rejected(_) => {
                    job.metadata.insert(JOB_METADATA_STATE_UPDATE_LAST_FAILED_BLOCK_NO.into(), block_no.to_string());
                    return Ok(tx_inclusion_status.into());
                }
                // If the tx is still pending, we wait for it to be finalized and check again the status.
                SettlementVerificationStatus::Pending => {
                    settlement_client.wait_for_tx_finality(tx_hash).await?;
                    let new_status = settlement_client.verify_tx_inclusion(tx_hash).await?;
                    match new_status {
                        SettlementVerificationStatus::Rejected(_) => {
                            job.metadata
                                .insert(JOB_METADATA_STATE_UPDATE_LAST_FAILED_BLOCK_NO.into(), block_no.to_string());
                            return Ok(new_status.into());
                        }
                        SettlementVerificationStatus::Pending => {
                            return Err(eyre!("Tx {tx_hash} should not be pending."))
                        }
                        SettlementVerificationStatus::Verified => {}
                    }
                }
                SettlementVerificationStatus::Verified => {}
            }
        }
        // verify that the last settled block is indeed the one we expect to be
        let expected_last_block_number = block_numbers.last().expect("Block numbers list should not be empty.");
        let out_last_block_number = settlement_client.get_last_settled_block().await?;
        let block_status = if out_last_block_number == *expected_last_block_number {
            SettlementVerificationStatus::Verified
        } else {
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
}

impl StateUpdateJob {
    /// Read the metadata and parse the block numbers
    fn get_block_numbers_from_metadata(&self, job: &JobItem) -> Result<Vec<u64>> {
        let blocks_to_settle = job.metadata.get(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY).ok_or_else(|| {
            eyre!("Block numbers to settle must be specified (state update job #{})", job.internal_id)
        })?;
        self.parse_block_numbers(blocks_to_settle)
    }

    /// Parse a list of blocks comma separated
    fn parse_block_numbers(&self, blocks_to_settle: &str) -> Result<Vec<u64>> {
        let sanitized_blocks = blocks_to_settle.replace(' ', "");
        let block_numbers: Vec<u64> = sanitized_blocks
            .split(',')
            .map(|block_no| block_no.parse::<u64>())
            .collect::<Result<Vec<u64>, _>>()
            .map_err(|e| eyre!("Block numbers to settle list is not correctly formatted: {e}"))?;
        Ok(block_numbers)
    }

    /// Validate that the list of block numbers to process is valid.
    async fn validate_block_numbers(&self, config: &Config, block_numbers: &[u64]) -> Result<()> {
        if block_numbers.is_empty() {
            return Err(eyre!("No block numbers found."));
        }
        if has_dup(block_numbers) {
            return Err(eyre!("Duplicated block numbers."));
        }
        if !is_sorted(block_numbers) {
            return Err(eyre!("Block numbers aren't sorted in increasing order."));
        }
        // Check for gap between the last settled block and the first block to settle
        let last_settled_block: u64 = config.settlement_client().get_last_settled_block().await?;
        if last_settled_block + 1 != block_numbers[0] {
            return Err(eyre!("Gap detected between the first block to settle and the last one settled."));
        }
        Ok(())
    }

    /// Update the state for the corresponding block using the settlement layer.
    async fn update_state_for_block(
        &self,
        config: &Config,
        block_no: u64,
        snos: StarknetOsOutput,
        fetch_from_tests: Option<bool>,
    ) -> Result<String> {
        let starknet_client = config.starknet_client();
        let settlement_client = config.settlement_client();
        let last_tx_hash_executed = if snos.use_kzg_da == Felt252::ZERO {
            let state_update = starknet_client.get_state_update(BlockId::Number(block_no)).await?;
            let _state_update = match state_update {
                MaybePendingStateUpdate::PendingUpdate(_) => {
                    return Err(eyre!("Block #{} - Cannot update state as it's still in pending state", block_no));
                }
                MaybePendingStateUpdate::Update(state_update) => state_update,
            };
            // TODO: Build the required arguments & send them to update_state_calldata
            let program_output = vec![];
            let onchain_data_hash = vec![0_u8; 32].try_into().expect("onchain data hash size must be 32 bytes");
            let onchain_data_size = 0;
            settlement_client.update_state_calldata(program_output, onchain_data_hash, onchain_data_size).await?
        } else if snos.use_kzg_da == Felt252::ONE {
            // TODO: Build the blob & the KZG proof & send them to update_state_blobs
            let kzg_proof = self.fetch_kzg_proof_for_block(block_no, fetch_from_tests).await;
            let kzg_proof: [u8; 48] = kzg_proof.try_into().expect("kzg proof size must be 48 bytes");
            settlement_client.update_state_blobs(vec![], kzg_proof).await?
        } else {
            return Err(eyre!("Block #{} - SNOS error, [use_kzg_da] should be either 0 or 1.", block_no));
        };
        Ok(last_tx_hash_executed)
    }

    /// Retrieves the SNOS output for the corresponding block.
    /// TODO: remove the fetch_from_tests argument once we have proper fetching (db/s3)
    async fn fetch_snos_for_block(&self, block_no: u64, fetch_from_tests: Option<bool>) -> StarknetOsOutput {
        let fetch_from_tests = fetch_from_tests.unwrap_or(true);
        match fetch_from_tests {
            true => {
                let snos_path =
                    CURRENT_PATH.join(format!("src/jobs/state_update_job/test_data/{}/snos_output.json", block_no));
                let snos_str = std::fs::read_to_string(snos_path).expect("Failed to read the SNOS json file");
                serde_json::from_str(&snos_str).expect("Failed to deserialize JSON into SNOS")
            }
            false => unimplemented!("can't fetch SNOS from DB/S3"),
        }
    }

    /// Retrieves the KZG Proof for the corresponding block.
    /// TODO: remove the fetch_from_tests argument once we have proper fetching (db/s3)
    async fn fetch_kzg_proof_for_block(&self, block_no: u64, fetch_from_tests: Option<bool>) -> Vec<u8> {
        let fetch_from_tests = fetch_from_tests.unwrap_or(true);
        let kzg_proof_str = match fetch_from_tests {
            true => {
                let kzg_path =
                    CURRENT_PATH.join(format!("src/jobs/state_update_job/test_data/{}/kzg_proof.txt", block_no));
                std::fs::read_to_string(kzg_path).expect("Failed to read the KZG txt file").replace("0x", "")
            }
            false => unimplemented!("can't fetch KZG Proof from DB/S3"),
        };
        hex::decode(kzg_proof_str).expect("Invalid test kzg proof")
    }

    /// Insert the tx hashes into the the metadata for the attempt number - will be used later by
    /// verify_job to make sure that all tx are successful.
    fn insert_attempts_into_metadata(&self, job: &mut JobItem, attempt_no: &str, tx_hashes: &[String]) {
        let new_attempt_metadata_key = format!("{}{}", JOB_METADATA_STATE_UPDATE_ATTEMPT_PREFIX, attempt_no);
        job.metadata.insert(new_attempt_metadata_key, tx_hashes.join(","));
    }
}
