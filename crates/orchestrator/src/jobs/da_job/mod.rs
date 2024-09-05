use std::collections::HashMap;
use std::ops::{Add, Mul, Rem};
use std::str::FromStr;

use async_trait::async_trait;
use color_eyre::eyre::WrapErr;
use lazy_static::lazy_static;
use num_bigint::{BigUint, ToBigUint};
use num_traits::{Num, Zero};
use starknet::core::types::{BlockId, FieldElement, MaybePendingStateUpdate, StateUpdate, StorageEntry};
use starknet::providers::Provider;
use thiserror::Error;
use tracing::log;
use uuid::Uuid;

use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::{Job, JobError, OtherError};
use crate::config::Config;
use crate::constants::BLOB_DATA_FILE_NAME;

lazy_static! {
    /// EIP-4844 BLS12-381 modulus.
    ///
    /// As defined in https://eips.ethereum.org/EIPS/eip-4844

    /// Generator of the group of evaluation points (EIP-4844 parameter).
    pub static ref GENERATOR: BigUint = BigUint::from_str(
        "39033254847818212395286706435128746857159659164139250548781411570340225835782",
    )
    .unwrap();

    pub static ref BLS_MODULUS: BigUint = BigUint::from_str(
        "52435875175126190479447740508185965837690552500527637822603658699938581184513",
    )
    .unwrap();
    pub static ref TWO: BigUint = 2u32.to_biguint().unwrap();

    pub static ref BLOB_LEN: usize = 4096;
}

#[derive(Error, Debug, PartialEq)]
pub enum DaError {
    #[error("Cannot process block {block_no:?} for job id {job_id:?} as it's still in pending state.")]
    BlockPending { block_no: String, job_id: Uuid },

    #[error("Blob size must be at least 32 bytes to accommodate a single FieldElement/BigUint, but was {blob_size:?}")]
    InsufficientBlobSize { blob_size: u64 },

    #[error("Exceeded the maximum number of blobs per transaction: allowed {max_blob_per_txn:?}, found {current_blob_length:?} for block {block_no:?} and job id {job_id:?}")]
    MaxBlobsLimitExceeded { max_blob_per_txn: u64, current_blob_length: u64, block_no: String, job_id: Uuid },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

pub struct DaJob;

#[async_trait]
impl Job for DaJob {
    async fn create_job(
        &self,
        _config: &Config,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError> {
        Ok(JobItem {
            id: Uuid::new_v4(),
            internal_id,
            job_type: JobType::DataSubmission,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
        })
    }

    async fn process_job(&self, config: &Config, job: &mut JobItem) -> Result<String, JobError> {
        let block_no = job
            .internal_id
            .parse::<u64>()
            .wrap_err("Failed to parse u64".to_string())
            .map_err(|e| JobError::Other(OtherError(e)))?;

        let state_update = config
            .starknet_client()
            .get_state_update(BlockId::Number(block_no))
            .await
            .wrap_err("Failed to get state Update.".to_string())
            .map_err(|e| JobError::Other(OtherError(e)))?;

        let state_update = match state_update {
            MaybePendingStateUpdate::PendingUpdate(_) => {
                Err(DaError::BlockPending { block_no: block_no.to_string(), job_id: job.id })?
            }
            MaybePendingStateUpdate::Update(state_update) => state_update,
        };
        // constructing the data from the rpc
        let blob_data = state_update_to_blob_data(block_no, state_update, config)
            .await
            .map_err(|e| JobError::Other(OtherError(e)))?;
        // transforming the data so that we can apply FFT on this.
        // @note: we can skip this step if in the above step we return vec<BigUint> directly
        let blob_data_biguint = convert_to_biguint(blob_data.clone());
        // data transformation on the data
        let transformed_data = fft_transformation(blob_data_biguint);

        let max_bytes_per_blob = config.da_client().max_bytes_per_blob().await;
        let max_blob_per_txn = config.da_client().max_blob_per_txn().await;

        // converting BigUints to Vec<u8>, one Vec<u8> represents one blob data
        let blob_array = data_to_blobs(max_bytes_per_blob, transformed_data)?;
        let current_blob_length: u64 = blob_array
            .len()
            .try_into()
            .wrap_err("Unable to convert the blob length into u64 format.".to_string())
            .map_err(|e| JobError::Other(OtherError(e)))?;

        // there is a limit on number of blobs per txn, checking that here
        if current_blob_length > max_blob_per_txn {
            Err(DaError::MaxBlobsLimitExceeded {
                max_blob_per_txn,
                current_blob_length,
                block_no: block_no.to_string(),
                job_id: job.id,
            })?
        }

        // making the txn to the DA layer
        let external_id = config
            .da_client()
            .publish_state_diff(blob_array, &[0; 32])
            .await
            .map_err(|e| JobError::Other(OtherError(e)))?;

        Ok(external_id)
    }

    async fn verify_job(&self, config: &Config, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        Ok(config
            .da_client()
            .verify_inclusion(job.external_id.unwrap_string().map_err(|e| JobError::Other(OtherError(e)))?)
            .await
            .map_err(|e| JobError::Other(OtherError(e)))?
            .into())
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

pub fn convert_to_biguint(elements: Vec<FieldElement>) -> Vec<BigUint> {
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

    // Convert all FieldElements to bytes first
    let mut bytes: Vec<u8> = block_data.iter().flat_map(|element| element.to_bytes_be().to_vec()).collect();

    // Process bytes in chunks of blob_size
    while bytes.len() >= blob_size as usize {
        let chunk = bytes.drain(..blob_size as usize).collect();
        blobs.push(chunk);
    }

    // Handle any remaining bytes (not a complete blob)
    if !bytes.is_empty() {
        let remaining_bytes = bytes.len();
        let mut last_blob = bytes;
        last_blob.resize(blob_size as usize, 0); // Pad with zeros
        blobs.push(last_blob);
        log::debug!("Warning: Remaining {} bytes not forming a complete blob were padded", remaining_bytes);
    }

    Ok(blobs)
}

pub async fn state_update_to_blob_data(
    block_no: u64,
    state_update: StateUpdate,
    config: &Config,
) -> color_eyre::Result<Vec<FieldElement>> {
    let state_diff = state_update.state_diff;
    let mut blob_data: Vec<FieldElement> = vec![
        FieldElement::from(state_diff.storage_diffs.len()),
        // @note: won't need this if while producing the block we are attaching the block number
        // and the block hash
        FieldElement::ONE,
        FieldElement::ONE,
        FieldElement::from(block_no),
        state_update.block_hash,
    ];

    let storage_diffs: HashMap<FieldElement, &Vec<StorageEntry>> =
        state_diff.storage_diffs.iter().map(|item| (item.address, &item.storage_entries)).collect();
    let declared_classes: HashMap<FieldElement, FieldElement> =
        state_diff.declared_classes.iter().map(|item| (item.class_hash, item.compiled_class_hash)).collect();
    let deployed_contracts: HashMap<FieldElement, FieldElement> =
        state_diff.deployed_contracts.iter().map(|item| (item.address, item.class_hash)).collect();
    let replaced_classes: HashMap<FieldElement, FieldElement> =
        state_diff.replaced_classes.iter().map(|item| (item.contract_address, item.class_hash)).collect();
    let mut nonces: HashMap<FieldElement, FieldElement> =
        state_diff.nonces.iter().map(|item| (item.contract_address, item.nonce)).collect();

    // Loop over storage diffs
    for (addr, writes) in storage_diffs {
        let class_flag = deployed_contracts.get(&addr).or_else(|| replaced_classes.get(&addr));

        let mut nonce = nonces.remove(&addr);

        // @note: if nonce is null and there is some len of writes, make an api call to get the contract
        // nonce for the block

        if nonce.is_none() && !writes.is_empty() && addr != FieldElement::ONE {
            let get_current_nonce_result = config
                .starknet_client()
                .get_nonce(BlockId::Number(block_no), addr)
                .await
                .wrap_err("Failed to get nonce ".to_string())?;

            nonce = Some(get_current_nonce_result);
        }
        let da_word = da_word(class_flag.is_some(), nonce, writes.len() as u64);
        // @note: it can be improved if the first push to the data is of block number and hash
        // @note: ONE address is special address which for now has 1 value and that is current
        //        block number and hash
        // @note: ONE special address can be used to mark the range of block, if in future
        //        the team wants to submit multiple blocks in a single blob etc.
        if addr == FieldElement::ONE && da_word == FieldElement::ONE {
            continue;
        }
        blob_data.push(addr);
        blob_data.push(da_word);

        if let Some(class_hash) = class_flag {
            blob_data.push(*class_hash);
        }

        for entry in writes {
            blob_data.push(entry.key);
            blob_data.push(entry.value);
        }
    }
    // Handle declared classes
    blob_data.push(FieldElement::from(declared_classes.len()));

    for (class_hash, compiled_class_hash) in &declared_classes {
        blob_data.push(*class_hash);
        blob_data.push(*compiled_class_hash);
    }

    // saving the blob data of the block to storage client
    store_blob_data(blob_data.clone(), block_no, config).await?;

    Ok(blob_data)
}

/// To store the blob data using the storage client with path <block_number>/blob_data.txt
async fn store_blob_data(blob_data: Vec<FieldElement>, block_number: u64, config: &Config) -> Result<(), JobError> {
    let storage_client = config.storage();
    let key = block_number.to_string() + "/" + BLOB_DATA_FILE_NAME;
    let data_blob_big_uint = convert_to_biguint(blob_data.clone());

    let blobs_array = data_to_blobs(config.da_client().max_bytes_per_blob().await, data_blob_big_uint)?;

    let blob = blobs_array.clone();

    // converting Vec<Vec<u8> into Vec<u8>
    let blob_vec_u8 = bincode::serialize(&blob)
        .wrap_err("Unable to Serialize blobs (Vec<Vec<u8> into Vec<u8>)".to_string())
        .map_err(|e| JobError::Other(OtherError(e)))?;

    if !blobs_array.is_empty() {
        storage_client.put_data(blob_vec_u8.into(), &key).await.map_err(|e| JobError::Other(OtherError(e)))?;
    }

    Ok(())
}

/// DA word encoding:
/// |---padding---|---class flag---|---new nonce---|---num changes---|
///     127 bits        1 bit           64 bits          64 bits
fn da_word(class_flag: bool, nonce_change: Option<FieldElement>, num_changes: u64) -> FieldElement {
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
        let bytes: [u8; 32] = nonce_change.expect("Not able to convert the nonce_change var into [u8; 32] type. Possible Error : Improper parameter length.").to_bytes_be();
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

    FieldElement::from_dec_str(&decimal_string).expect("issue while converting to fieldElement")
}

#[cfg(test)]

pub mod test {
    use crate::jobs::da_job::da_word;
    use std::fs;
    use std::fs::File;
    use std::io::Read;
    use std::sync::Arc;

    use crate::config::config;
    use crate::data_storage::MockDataStorage;
    use crate::tests::config::TestConfigBuilder;
    use ::serde::{Deserialize, Serialize};
    use color_eyre::Result;
    use da_client_interface::MockDaClient;
    use httpmock::prelude::*;
    use majin_blob_core::blob;
    use majin_blob_types::serde;
    use majin_blob_types::state_diffs::UnorderedEq;
    use rstest::rstest;
    use serde_json::json;
    use starknet::providers::jsonrpc::HttpTransport;
    use starknet::providers::JsonRpcClient;
    use starknet_core::types::{FieldElement, StateUpdate};
    use url::Url;

    /// Tests `da_word` function with various inputs for class flag, new nonce, and number of changes.
    /// Verifies that `da_word` produces the correct FieldElement based on the provided parameters.
    /// Uses test cases with different combinations of inputs and expected output strings.
    /// Asserts the function's correctness by comparing the computed and expected FieldElements.
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
        let new_nonce = if new_nonce > 0 { Some(FieldElement::from(new_nonce)) } else { None };
        let da_word = da_word(class_flag, new_nonce, num_changes);
        let expected = FieldElement::from_dec_str(expected.as_str()).unwrap();
        assert_eq!(da_word, expected);
    }

    /// Tests `state_update_to_blob_data` conversion with different state update files and block numbers.
    /// Mocks DA client and storage client interactions for the test environment.
    /// Compares the generated blob data against expected values to ensure correctness.
    /// Verifies the data integrity by checking that the parsed state diffs match the expected diffs.
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
    #[tokio::test]
    async fn test_state_update_to_blob_data(
        #[case] block_no: u64,
        #[case] state_update_file_path: &str,
        #[case] file_path: &str,
        #[case] nonce_file_path: &str,
    ) {
        use crate::jobs::da_job::{convert_to_biguint, state_update_to_blob_data};

        let server = MockServer::start();
        let mut da_client = MockDaClient::new();
        let mut storage_client = MockDataStorage::new();

        // Mocking DA client calls
        da_client.expect_max_blob_per_txn().with().returning(|| 6);
        da_client.expect_max_bytes_per_blob().with().returning(|| 131072);

        // Mocking storage client
        storage_client.expect_put_data().returning(|_, _| Result::Ok(())).times(1);

        let provider = JsonRpcClient::new(HttpTransport::new(
            Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
        ));

        // mock block number (madara) : 5
        TestConfigBuilder::new()
            .mock_starknet_client(Arc::new(provider))
            .mock_da_client(Box::new(da_client))
            .mock_storage_client(Box::new(storage_client))
            .build()
            .await;

        let config = config().await;

        get_nonce_attached(&server, nonce_file_path);

        let state_update = read_state_update_from_file(state_update_file_path).expect("issue while reading");
        let blob_data = state_update_to_blob_data(block_no, state_update, &config)
            .await
            .expect("issue while converting state update to blob data");

        let blob_data_biguint = convert_to_biguint(blob_data);

        let block_data_state_diffs = serde::parse_state_diffs(blob_data_biguint.as_slice());

        let original_blob_data = serde::parse_file_to_blob_data(file_path);
        // converting the data to it's original format
        let recovered_blob_data = blob::recover(original_blob_data.clone());
        let blob_data_state_diffs = serde::parse_state_diffs(recovered_blob_data.as_slice());

        assert!(block_data_state_diffs.unordered_eq(&blob_data_state_diffs), "value of data json should be identical");
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
                FieldElement::from_dec_str(&address).expect("issue while converting the hex to field").to_bytes_be();
            let hex_field_element = vec_u8_to_hex_string(&field_element);

            server.mock(|when, then| {
                when.path("/").body_contains("starknet_getNonce").body_contains(hex_field_element);
                then.status(200).body(serde_json::to_vec(&response).unwrap());
            });
        }
    }

    fn vec_u8_to_hex_string(data: &[u8]) -> String {
        let hex_chars: Vec<String> = data.iter().map(|byte| format!("{:02x}", byte)).collect();

        let mut new_hex_chars = hex_chars.join("");
        new_hex_chars = new_hex_chars.trim_start_matches('0').to_string();
        if new_hex_chars.is_empty() {
            "0x0".to_string()
        } else {
            format!("0x{}", new_hex_chars)
        }
    }
}
