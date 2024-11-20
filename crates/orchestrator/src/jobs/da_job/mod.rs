use std::collections::{HashMap, HashSet};
use std::ops::{Add, Mul, Rem};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::WrapErr;
use lazy_static::lazy_static;
use num_bigint::{BigUint, ToBigUint};
use num_traits::{Num, Zero};
use starknet::core::types::{
    BlockId, ContractStorageDiffItem, DeclaredClassItem, Felt, MaybePendingStateUpdate, StateDiff, StateUpdate,
};
use starknet::providers::Provider;
use thiserror::Error;
use uuid::Uuid;

use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::{Job, JobError, OtherError};
use crate::config::Config;
use crate::constants::BLOB_DATA_FILE_NAME;
use crate::jobs::state_update_job::utils::biguint_vec_to_u8_vec;

lazy_static! {
    /// EIP-4844 BLS12-381 modulus.
    ///
    /// As defined in https://eips.ethereum.org/EIPS/eip-4844

    /// Generator of the group of evaluation points (EIP-4844 parameter).
    pub static ref GENERATOR: BigUint = BigUint::from_str(
        "39033254847818212395286706435128746857159659164139250548781411570340225835782",
    )
    .expect("Failed to convert to biguint");

    pub static ref BLS_MODULUS: BigUint = BigUint::from_str(
        "52435875175126190479447740508185965837690552500527637822603658699938581184513",
    )
    .expect("Failed to convert to biguint");
    pub static ref TWO: BigUint = 2u32.to_biguint().expect("Failed to convert to biguint");

    pub static ref BLOB_LEN: usize = 4096;
}

#[derive(Error, Debug, PartialEq)]
pub enum DaError {
    #[error("Cannot process block {block_no:?} for job id {job_id:?} as it's still in pending state.")]
    BlockPending { block_no: String, job_id: Uuid },

    #[error("Blob size must be at least 32 bytes to accommodate a single FieldElement/BigUint, but was {blob_size:?}")]
    InsufficientBlobSize { blob_size: u64 },

    #[error(
        "Exceeded the maximum number of blobs per transaction: allowed {max_blob_per_txn:?}, found \
         {current_blob_length:?} for block {block_no:?} and job id {job_id:?}"
    )]
    MaxBlobsLimitExceeded { max_blob_per_txn: u64, current_blob_length: u64, block_no: String, job_id: Uuid },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

pub struct DaJob;

#[async_trait]
impl Job for DaJob {
    #[tracing::instrument(fields(category = "da"), skip(self, _config, metadata), ret, err)]
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError> {
        let job_id = Uuid::new_v4();
        tracing::info!(log_type = "starting", category = "da", function_type = "create_job",  block_no = %internal_id, "DA job creation started.");
        let job_item = JobItem {
            id: job_id,
            internal_id: internal_id.clone(),
            job_type: JobType::DataSubmission,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        };
        tracing::info!(log_type = "completed", category = "da", function_type = "create_job", block_no = %internal_id, "DA job creation completed.");
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "da"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(log_type = "starting", category = "da", function_type = "process_job", job_id = ?job.id,  block_no = %internal_id, "DA job processing started.");
        let block_no = job.internal_id.parse::<u64>().wrap_err("Failed to parse u64".to_string()).map_err(|e| {
            tracing::error!(job_id = ?job.id, error = ?e, "Failed to parse block number");
            JobError::Other(OtherError(e))
        })?;

        let state_update = config
            .starknet_client()
            .get_state_update(BlockId::Number(block_no))
            .await
            .wrap_err("Failed to get state Update.".to_string())
            .map_err(|e| {
                tracing::error!(job_id = ?job.id, error = ?e, "Failed to get state update");
                JobError::Other(OtherError(e))
            })?;

        let state_update = match state_update {
            MaybePendingStateUpdate::PendingUpdate(_) => {
                tracing::warn!(job_id = ?job.id, block_no = block_no, "Block is still pending");
                Err(DaError::BlockPending { block_no: block_no.to_string(), job_id: job.id })?
            }
            MaybePendingStateUpdate::Update(state_update) => state_update,
        };
        tracing::debug!(job_id = ?job.id, "Retrieved state update");
        // constructing the data from the rpc
        let blob_data = state_update_to_blob_data(block_no, state_update, config.clone()).await.map_err(|e| {
            tracing::error!(job_id = ?job.id, error = ?e, "Failed to convert state update to blob data");
            JobError::Other(OtherError(e))
        })?;
        // transforming the data so that we can apply FFT on this.
        // @note: we can skip this step if in the above step we return vec<BigUint> directly
        let blob_data_biguint = convert_to_biguint(blob_data.clone());
        tracing::trace!(job_id = ?job.id, "Converted blob data to BigUint");

        let transformed_data = fft_transformation(blob_data_biguint);
        // data transformation on the data
        tracing::trace!(job_id = ?job.id, "Applied FFT transformation");

        store_blob_data(transformed_data.clone(), block_no, config.clone()).await?;
        tracing::debug!(job_id = ?job.id, "Stored blob data");

        let max_bytes_per_blob = config.da_client().max_bytes_per_blob().await;
        let max_blob_per_txn = config.da_client().max_blob_per_txn().await;
        tracing::trace!(job_id = ?job.id, max_bytes_per_blob = max_bytes_per_blob, max_blob_per_txn = max_blob_per_txn, "Retrieved DA client configuration");
        // converting BigUints to Vec<u8>, one Vec<u8> represents one blob data

        let blob_array = data_to_blobs(max_bytes_per_blob, transformed_data)?;
        let current_blob_length: u64 = blob_array
            .len()
            .try_into()
            .wrap_err("Unable to convert the blob length into u64 format.".to_string())
            .map_err(|e| {
                tracing::error!(job_id = ?job.id, error = ?e, "Failed to convert blob length to u64");
                JobError::Other(OtherError(e))
            })?;
        tracing::debug!(job_id = ?job.id, blob_count = current_blob_length, "Converted data to blobs");

        // there is a limit on number of blobs per txn, checking that here
        if current_blob_length > max_blob_per_txn {
            tracing::warn!(job_id = ?job.id, current_blob_length = current_blob_length, max_blob_per_txn = max_blob_per_txn, "Exceeded maximum number of blobs per transaction");
            Err(DaError::MaxBlobsLimitExceeded {
                max_blob_per_txn,
                current_blob_length,
                block_no: block_no.to_string(),
                job_id: job.id,
            })?
        }

        // making the txn to the DA layer
        let external_id = config.da_client().publish_state_diff(blob_array, &[0; 32]).await.map_err(|e| {
            tracing::error!(job_id = ?job.id, error = ?e, "Failed to publish state diff to DA layer");
            JobError::Other(OtherError(e))
        })?;

        tracing::info!(log_type = "completed", category = "da", function_type = "process_job", job_id = ?job.id,  block_no = %internal_id, external_id = ?external_id, "Successfully published state diff to DA layer.");
        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "da"), skip(self, config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(log_type = "starting", category = "da", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id, "DA job verification started.");
        let verification_status = config
            .da_client()
            .verify_inclusion(job.external_id.unwrap_string().map_err(|e| {
                tracing::error!(job_id = ?job.id, error = ?e, "Failed to unwrap external ID");
                JobError::Other(OtherError(e))
            })?)
            .await
            .map_err(|e| {
                tracing::error!(job_id = ?job.id, error = ?e, "Job verification failed");
                JobError::Other(OtherError(e))
            })?
            .into();

        tracing::info!(log_type = "completed", category = "da", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id, verification_status = ?verification_status, "DA job verification completed.");
        Ok(verification_status)
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        3
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        60
    }
}

#[tracing::instrument(skip(elements))]
pub fn fft_transformation(elements: Vec<BigUint>) -> Vec<BigUint> {
    let xs: Vec<BigUint> = (0..*BLOB_LEN)
        .map(|i| {
            let bin = format!("{:012b}", i);
            let bin_rev = bin.chars().rev().collect::<String>();
            GENERATOR.modpow(
                &BigUint::from_str_radix(&bin_rev, 2).expect("Not able to convert the parameters into exponent."),
                &BLS_MODULUS,
            )
        })
        .collect();
    let n = elements.len();
    let mut transform: Vec<BigUint> = vec![BigUint::zero(); n];

    for i in 0..n {
        for j in (0..n).rev() {
            transform[i] = (transform[i].clone().mul(&xs[i]).add(&elements[j])).rem(&*BLS_MODULUS);
        }
    }
    transform
}

pub fn convert_to_biguint(elements: Vec<Felt>) -> Vec<BigUint> {
    // Initialize the vector with 4096 BigUint zeros
    let mut biguint_vec = vec![BigUint::zero(); 4096];

    // Iterate over the elements and replace the zeros in the biguint_vec
    for (i, element) in elements.iter().take(4096).enumerate() {
        // Convert FieldElement to [u8; 32]
        let bytes: [u8; 32] = element.to_bytes_be();

        // Convert [u8; 32] to BigUint
        let biguint = BigUint::from_bytes_be(&bytes);

        // Replace the zero with the converted value
        biguint_vec[i] = biguint;
    }

    biguint_vec
}

fn data_to_blobs(blob_size: u64, block_data: Vec<BigUint>) -> Result<Vec<Vec<u8>>, JobError> {
    // Validate blob size
    if blob_size < 32 {
        Err(DaError::InsufficientBlobSize { blob_size })?
    }

    let mut blobs: Vec<Vec<u8>> = Vec::new();

    // Convert all BigUint to bytes
    let bytes: Vec<u8> = block_data.into_iter().flat_map(|num| num.to_bytes_be()).collect();

    // Process bytes in chunks of blob_size
    let chunk_size = blob_size as usize;
    let chunks = bytes.chunks(chunk_size);

    for chunk in chunks {
        let mut blob = chunk.to_vec();
        if blob.len() < chunk_size {
            blob.resize(chunk_size, 0);
            tracing::debug!("Warning: Last chunk of {} bytes was padded to full blob size", chunk.len());
        }
        blobs.push(blob);
    }

    Ok(blobs)
}

pub async fn state_update_to_blob_data(
    block_no: u64,
    state_update: StateUpdate,
    config: Arc<Config>,
) -> color_eyre::Result<Vec<Felt>> {
    let mut state_diff = state_update.state_diff;
    refactor_state_update(&mut state_diff);

    let mut blob_data: Vec<Felt> = vec![Felt::from(state_diff.storage_diffs.len())];

    let deployed_contracts: HashMap<Felt, Felt> =
        state_diff.deployed_contracts.into_iter().map(|item| (item.address, item.class_hash)).collect();
    let replaced_classes: HashMap<Felt, Felt> =
        state_diff.replaced_classes.into_iter().map(|item| (item.contract_address, item.class_hash)).collect();
    let mut nonces: HashMap<Felt, Felt> =
        state_diff.nonces.into_iter().map(|item| (item.contract_address, item.nonce)).collect();

    // sort storage diffs
    state_diff.storage_diffs.sort_by_key(|diff| diff.address);

    // Loop over storage diffs
    for ContractStorageDiffItem { address, mut storage_entries } in state_diff.storage_diffs.into_iter() {
        let class_flag = deployed_contracts.get(&address).or_else(|| replaced_classes.get(&address));

        let mut nonce = nonces.remove(&address);

        // @note: if nonce is null and there is some len of writes, make an api call to get the contract
        // nonce for the block

        if nonce.is_none() && !storage_entries.is_empty() && address != Felt::ONE {
            let get_current_nonce_result = config
                .starknet_client()
                .get_nonce(BlockId::Number(block_no), address)
                .await
                .wrap_err("Failed to get nonce ".to_string())?;

            nonce = Some(get_current_nonce_result);
        }
        let da_word = da_word(class_flag.is_some(), nonce, storage_entries.len() as u64);
        blob_data.push(address);
        blob_data.push(da_word);

        if let Some(class_hash) = class_flag {
            blob_data.push(*class_hash);
        }

        storage_entries.sort_by_key(|entry| entry.key);
        for entry in storage_entries {
            blob_data.push(entry.key);
            blob_data.push(entry.value);
        }
    }
    // Handle declared classes
    blob_data.push(Felt::from(state_diff.declared_classes.len()));

    // sort storage diffs
    state_diff.declared_classes.sort_by_key(|class| class.class_hash);

    for DeclaredClassItem { class_hash, compiled_class_hash } in state_diff.declared_classes.into_iter() {
        blob_data.push(class_hash);
        blob_data.push(compiled_class_hash);
    }

    Ok(blob_data)
}

/// To store the blob data using the storage client with path <block_number>/blob_data.txt
async fn store_blob_data(blob_data: Vec<BigUint>, block_number: u64, config: Arc<Config>) -> Result<(), JobError> {
    let storage_client = config.storage();
    let key = block_number.to_string() + "/" + BLOB_DATA_FILE_NAME;

    let blob_data_vec_u8 = biguint_vec_to_u8_vec(blob_data.as_slice());

    if !blob_data_vec_u8.is_empty() {
        storage_client.put_data(blob_data_vec_u8.into(), &key).await.map_err(|e| JobError::Other(OtherError(e)))?;
    }

    Ok(())
}

/// DA word encoding:
/// |---padding---|---class flag---|---new nonce---|---num changes---|
///     127 bits        1 bit           64 bits          64 bits
fn da_word(class_flag: bool, nonce_change: Option<Felt>, num_changes: u64) -> Felt {
    // padding of 127 bits
    let mut binary_string = "0".repeat(127);

    // class flag of one bit
    if class_flag {
        binary_string += "1"
    } else {
        binary_string += "0"
    }

    // checking for nonce here
    if let Some(_new_nonce) = nonce_change {
        let bytes: [u8; 32] = nonce_change
            .expect(
                "Not able to convert the nonce_change var into [u8; 32] type. Possible Error : Improper parameter \
                 length.",
            )
            .to_bytes_be();
        let biguint = BigUint::from_bytes_be(&bytes);
        let binary_string_local = format!("{:b}", biguint);
        let padded_binary_string = format!("{:0>64}", binary_string_local);
        binary_string += &padded_binary_string;
    } else {
        let binary_string_local = "0".repeat(64);
        binary_string += &binary_string_local;
    }

    let binary_representation = format!("{:b}", num_changes);
    let padded_binary_string = format!("{:0>64}", binary_representation);
    binary_string += &padded_binary_string;

    let biguint = BigUint::from_str_radix(binary_string.as_str(), 2).expect("Invalid binary string");

    // Now convert the BigUint to a decimal string
    let decimal_string = biguint.to_str_radix(10);

    Felt::from_dec_str(&decimal_string).expect("issue while converting to fieldElement")
}

fn refactor_state_update(state_update: &mut StateDiff) {
    let existing_storage: HashSet<_> = state_update.storage_diffs.iter().map(|item| item.address).collect();

    // Collect new addresses, using a HashSet for deduplication
    let new_addresses: HashSet<_> = Iterator::chain(
        state_update.nonces.iter().map(|item| item.contract_address),
        state_update.deployed_contracts.iter().map(|item| item.address),
    )
    .filter(|address| !existing_storage.contains(address))
    .collect();

    // Add new storage diffs in batch
    state_update.storage_diffs.extend(
        new_addresses.into_iter().map(|address| ContractStorageDiffItem { address, storage_entries: Vec::new() }),
    );
}

#[cfg(test)]
pub mod test {
    use std::collections::HashSet;
    use std::fs;
    use std::fs::File;
    use std::io::Read;

    use ::serde::{Deserialize, Serialize};
    use color_eyre::Result;
    use da_client_interface::MockDaClient;
    use httpmock::prelude::*;
    use majin_blob_core::blob;
    use majin_blob_types::serde;
    use rstest::rstest;
    use serde_json::json;
    use starknet::core::types::{
        ContractStorageDiffItem, DeployedContractItem, Felt, NonceUpdate, StateDiff, StateUpdate, StorageEntry,
    };
    use starknet::providers::jsonrpc::HttpTransport;
    use starknet::providers::JsonRpcClient;
    use url::Url;

    use crate::jobs::da_job::{da_word, refactor_state_update};

    /// Tests `da_word` function with various inputs for class flag, new nonce, and number of
    /// changes. Verifies that `da_word` produces the correct Felt based on the provided
    /// parameters. Uses test cases with different combinations of inputs and expected output
    /// strings. Asserts the function's correctness by comparing the computed and expected
    /// Felts.
    #[rstest]
    #[case(false, 1, 1, "18446744073709551617")]
    #[case(false, 1, 0, "18446744073709551616")]
    #[case(false, 0, 6, "6")]
    #[case(true, 1, 0, "340282366920938463481821351505477763072")]
    fn test_da_word(
        #[case] class_flag: bool,
        #[case] new_nonce: u64,
        #[case] num_changes: u64,
        #[case] expected: String,
    ) {
        let new_nonce = if new_nonce > 0 { Some(Felt::from(new_nonce)) } else { None };
        let da_word = da_word(class_flag, new_nonce, num_changes);
        let expected = Felt::from_dec_str(expected.as_str()).unwrap();
        assert_eq!(da_word, expected);
    }

    /// Tests `state_update_to_blob_data` conversion with different state update files and block
    /// numbers. Mocks DA client and storage client interactions for the test environment.
    /// Compares the generated blob data against expected values to ensure correctness.
    /// Verifies the data integrity by checking that the parsed state diffs match the expected
    /// diffs.
    #[rstest]
    #[case(
        631861,
        "src/tests/jobs/da_job/test_data/state_update/631861.txt",
        "src/tests/jobs/da_job/test_data/test_blob/631861.txt",
        "src/tests/jobs/da_job/test_data/nonces/631861.txt"
    )]
    #[case(
        638353,
        "src/tests/jobs/da_job/test_data/state_update/638353.txt",
        "src/tests/jobs/da_job/test_data/test_blob/638353.txt",
        "src/tests/jobs/da_job/test_data/nonces/638353.txt"
    )]
    #[case(
        640641,
        "src/tests/jobs/da_job/test_data/state_update/640641.txt",
        "src/tests/jobs/da_job/test_data/test_blob/640641.txt",
        "src/tests/jobs/da_job/test_data/nonces/640641.txt"
    )]
    #[case(
        671070,
        "src/tests/jobs/da_job/test_data/state_update/671070.txt",
        "src/tests/jobs/da_job/test_data/test_blob/671070.txt",
        "src/tests/jobs/da_job/test_data/nonces/671070.txt"
    )]
    // Block from pragma madara and orch test run. Here we faced an issue where our
    // blob building logic was not able to take the contract addresses from
    // `deployed_contracts` field in state diff from state update. This test case
    // was added after the fix
    #[case(
        178,
        "src/tests/jobs/da_job/test_data/state_update/178.txt",
        "src/tests/jobs/da_job/test_data/test_blob/178.txt",
        "src/tests/jobs/da_job/test_data/nonces/178.txt"
    )]
    #[tokio::test]
    async fn test_state_update_to_blob_data(
        #[case] block_no: u64,
        #[case] state_update_file_path: &str,
        #[case] file_path: &str,
        #[case] nonce_file_path: &str,
    ) {
        use crate::jobs::da_job::{convert_to_biguint, state_update_to_blob_data};
        use crate::tests::config::TestConfigBuilder;

        let server = MockServer::start();
        let mut da_client = MockDaClient::new();

        // Mocking DA client calls
        da_client.expect_max_blob_per_txn().with().returning(|| 6);
        da_client.expect_max_bytes_per_blob().with().returning(|| 131072);

        // Mocking storage client
        let provider = JsonRpcClient::new(HttpTransport::new(
            Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
        ));

        // mock block number (madara) : 5
        let services = TestConfigBuilder::new()
            .configure_starknet_client(provider.into())
            .configure_da_client(da_client.into())
            .build()
            .await;

        get_nonce_attached(&server, nonce_file_path);

        let state_update = read_state_update_from_file(state_update_file_path).expect("issue while reading");
        let blob_data = state_update_to_blob_data(block_no, state_update, services.config)
            .await
            .expect("issue while converting state update to blob data");
        let blob_data_biguint = convert_to_biguint(blob_data);

        let original_blob_data = serde::parse_file_to_blob_data(file_path);
        // converting the data to it's original format
        let recovered_blob_data = blob::recover(original_blob_data.clone());

        assert_eq!(blob_data_biguint, recovered_blob_data);
    }

    /// Tests the `fft_transformation` function with various test blob files.
    /// Verifies the correctness of FFT and IFFT transformations by ensuring round-trip consistency.
    /// Parses the original blob data, recovers it using IFFT, and re-applies FFT.
    /// Asserts that the transformed data matches the original pre-IFFT data, ensuring integrity.
    #[rstest]
    #[case("src/tests/jobs/da_job/test_data/test_blob/638353.txt")]
    #[case("src/tests/jobs/da_job/test_data/test_blob/631861.txt")]
    #[case("src/tests/jobs/da_job/test_data/test_blob/639404.txt")]
    #[case("src/tests/jobs/da_job/test_data/test_blob/640641.txt")]
    #[case("src/tests/jobs/da_job/test_data/test_blob/640644.txt")]
    #[case("src/tests/jobs/da_job/test_data/test_blob/640646.txt")]
    #[case("src/tests/jobs/da_job/test_data/test_blob/640647.txt")]
    fn test_fft_transformation(#[case] file_to_check: &str) {
        // parsing the blob hex to the bigUints

        use crate::jobs::da_job::fft_transformation;
        let original_blob_data = serde::parse_file_to_blob_data(file_to_check);
        // converting the data to its original format
        let ifft_blob_data = blob::recover(original_blob_data.clone());
        // applying the fft function again on the original format
        let fft_blob_data = fft_transformation(ifft_blob_data);

        // ideally the data after fft transformation and the data before ifft should be same.
        assert_eq!(fft_blob_data, original_blob_data);
    }

    /// Tests the serialization and deserialization process using bincode.
    /// Serializes a nested vector of integers and then deserializes it back.
    /// Verifies that the original data matches the deserialized data.
    /// Ensures the integrity and correctness of bincode's (de)serialization.
    #[rstest]
    fn test_bincode() {
        let data = vec![vec![1, 2], vec![3, 4]];

        let serialize_data = bincode::serialize(&data).unwrap();
        let deserialize_data: Vec<Vec<u8>> = bincode::deserialize(&serialize_data).unwrap();

        assert_eq!(data, deserialize_data);
    }

    #[rstest]
    #[case::empty_case(vec![], vec![], vec![], 0)]
    #[case::only_nonces(
        vec![(Felt::from(1), Felt::from(10)), (Felt::from(2), Felt::from(20))],
        vec![],
        vec![],
        2
    )]
    #[case::only_deployed(
        vec![],
        vec![],
        vec![(Felt::from(1), vec![1]), (Felt::from(2), vec![2])],
        2
    )]
    #[case::overlapping_addresses(
        vec![(Felt::from(1), Felt::from(10))],
        vec![(Felt::from(1), vec![(Felt::from(1), Felt::from(100))])],
        vec![(Felt::from(1), vec![1])],
        1
    )]
    #[case::duplicate_addresses(
        vec![(Felt::from(1), Felt::from(10)), (Felt::from(1), Felt::from(20))],
        vec![],
        vec![(Felt::from(1), vec![1]), (Felt::from(1), vec![2])],
        1
    )]
    fn test_refactor_state_update(
        #[case] nonces: Vec<(Felt, Felt)>,
        #[case] storage_diffs: Vec<(Felt, Vec<(Felt, Felt)>)>,
        #[case] deployed_contracts: Vec<(Felt, Vec<u8>)>,
        #[case] expected_storage_count: usize,
    ) {
        let mut state_diff = create_state_diff(nonces, storage_diffs.clone(), deployed_contracts);
        let initial_storage = state_diff.storage_diffs.clone();

        refactor_state_update(&mut state_diff);

        assert!(verify_addresses_have_storage_diffs(&state_diff, &initial_storage));
        verify_unique_addresses(&state_diff, expected_storage_count);
    }

    pub(crate) fn read_state_update_from_file(file_path: &str) -> Result<StateUpdate> {
        // let file_path = format!("state_update_block_no_{}.txt", block_no);
        let mut file = File::open(file_path)?;
        let mut json = String::new();
        file.read_to_string(&mut json)?;
        let state_update: StateUpdate = serde_json::from_str(&json)?;
        Ok(state_update)
    }

    #[derive(Serialize, Deserialize, Debug)]
    struct NonceAddress {
        nonce: String,
        address: String,
    }

    pub fn get_nonce_attached(server: &MockServer, file_path: &str) {
        // Read the file
        let file_content = fs::read_to_string(file_path).expect("Unable to read file");

        // Parse the JSON content into a vector of NonceAddress
        let nonce_addresses: Vec<NonceAddress> =
            serde_json::from_str(&file_content).expect("JSON was not well-formatted");

        // Set up mocks for each entry
        for entry in nonce_addresses {
            let address = entry.address.clone();
            let nonce = entry.nonce.clone();
            let response = json!({ "id": 1,"jsonrpc":"2.0","result": nonce });
            let field_element =
                Felt::from_dec_str(&address).expect("issue while converting the hex to field").to_bytes_be();
            let hex_field_element = vec_u8_to_hex_string(&field_element);

            server.mock(|when, then| {
                when.path("/").body_includes("starknet_getNonce").body_includes(hex_field_element);
                then.status(200).body(serde_json::to_vec(&response).unwrap());
            });
        }
    }

    fn vec_u8_to_hex_string(data: &[u8]) -> String {
        let hex_chars: Vec<String> = data.iter().map(|byte| format!("{:02x}", byte)).collect();

        let mut new_hex_chars = hex_chars.join("");
        new_hex_chars = new_hex_chars.trim_start_matches('0').to_string();
        if new_hex_chars.is_empty() { "0x0".to_string() } else { format!("0x{}", new_hex_chars) }
    }

    fn create_state_diff(
        nonces: Vec<(Felt, Felt)>,
        storage_diffs: Vec<(Felt, Vec<(Felt, Felt)>)>,
        deployed_contracts: Vec<(Felt, Vec<u8>)>,
    ) -> StateDiff {
        StateDiff {
            nonces: nonces.into_iter().map(|(addr, nonce)| NonceUpdate { contract_address: addr, nonce }).collect(),
            storage_diffs: storage_diffs
                .into_iter()
                .map(|(addr, entries)| ContractStorageDiffItem {
                    address: addr,
                    storage_entries: entries.into_iter().map(|(key, value)| StorageEntry { key, value }).collect(),
                })
                .collect(),
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: deployed_contracts
                .into_iter()
                .map(|(addr, _class_hash)| DeployedContractItem { address: addr, class_hash: Default::default() })
                .collect(),
            replaced_classes: vec![],
        }
    }

    fn verify_unique_addresses(state_diff: &StateDiff, expected_count: usize) {
        let unique_addresses: HashSet<_> = state_diff.storage_diffs.iter().map(|item| &item.address).collect();

        assert_eq!(unique_addresses.len(), state_diff.storage_diffs.len(), "Storage diffs contain duplicate addresses");
        assert_eq!(unique_addresses.len(), expected_count, "Unexpected number of storage diffs");
    }

    fn verify_addresses_have_storage_diffs(
        state_diff: &StateDiff,
        initial_storage: &Vec<ContractStorageDiffItem>,
    ) -> bool {
        for orig_storage in initial_storage {
            if let Some(current_storage) =
                state_diff.storage_diffs.iter().find(|item| item.address == orig_storage.address)
            {
                assert_eq!(
                    orig_storage.storage_entries, current_storage.storage_entries,
                    "Storage entries changed unexpectedly"
                );
            }
        }

        let storage_addresses: HashSet<_> = state_diff.storage_diffs.iter().map(|item| &item.address).collect();

        state_diff.nonces.iter().all(|item| storage_addresses.contains(&item.contract_address))
            && state_diff.deployed_contracts.iter().all(|item| storage_addresses.contains(&item.address))
    }
}
