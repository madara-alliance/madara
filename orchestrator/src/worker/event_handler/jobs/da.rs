use crate::compression::blob::da_word;
use crate::core::config::Config;
use crate::error::job::da_error::DaError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::constant::{BLOB_LEN, BLS_MODULUS, GENERATOR};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{DaMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::biguint_vec_to_u8_vec;
use async_trait::async_trait;
use color_eyre::eyre::{eyre, Context};
use num_bigint::BigUint;
use num_traits::{Num, Zero};
use starknet::providers::Provider;
use starknet_core::types::{
    BlockId, ContractStorageDiffItem, DeclaredClassItem, Felt, MaybePreConfirmedStateUpdate, StateDiff, StateUpdate,
};
use std::collections::{HashMap, HashSet};
use std::ops::{Add, Mul, Rem};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

pub struct DAJobHandler;

impl DAJobHandler {
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

    pub fn fft_transformation(elements: Vec<BigUint>) -> Result<Vec<BigUint>, JobError> {
        let xs: Vec<BigUint> = (0..BLOB_LEN)
            .map(|i| {
                let bin = format!("{:012b}", i);
                let bin_rev = bin.chars().rev().collect::<String>();
                let exponent = BigUint::from_str_radix(&bin_rev, 2)
                    .wrap_err("Failed to convert binary string to exponent")
                    .map_err(|e| JobError::Other(OtherError(e)))?;
                Ok(GENERATOR.modpow(&exponent, &BLS_MODULUS))
            })
            .collect::<Result<Vec<BigUint>, JobError>>()?;

        let n = elements.len();
        let mut transform: Vec<BigUint> = vec![BigUint::zero(); n];

        for i in 0..n {
            for j in (0..n).rev() {
                transform[i] = (transform[i].clone().mul(&xs[i]).add(&elements[j])).rem(&*BLS_MODULUS);
            }
        }
        Ok(transform)
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

    pub async fn state_update_to_blob_data(
        block_no: u64,
        state_update: StateUpdate,
        config: Arc<Config>,
    ) -> Result<Vec<Felt>, JobError> {
        let mut state_diff = state_update.state_diff;
        Self::refactor_state_update(&mut state_diff);

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

            if nonce.is_none() && !storage_entries.is_empty() && address != Felt::ONE && address != Felt::TWO {
                let get_current_nonce_result =
                    config.madara_rpc_client().get_nonce(BlockId::Number(block_no), address).await.map_err(|e| {
                        JobError::ProviderError(format!(
                            "Failed to get nonce for address {address} at block {block_no}: {e}"
                        ))
                    })?;

                nonce = Some(get_current_nonce_result);
            }
            let da_word =
                da_word(class_flag.is_some(), nonce, storage_entries.len() as u64, config.params.madara_version)?;
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
            }
            blobs.push(blob);
        }

        Ok(blobs)
    }

    /// To store the blob data using the storage client with path <block_number>/blob_data.txt
    async fn store_blob_data(
        blob_data: Vec<BigUint>,
        blob_data_path: &str,
        config: Arc<Config>,
    ) -> Result<(), JobError> {
        let storage_client = config.storage();

        let blob_data_vec_u8 = biguint_vec_to_u8_vec(blob_data.as_slice());

        if !blob_data_vec_u8.is_empty() {
            storage_client.put_data(blob_data_vec_u8.into(), blob_data_path).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl JobHandlerTrait for DAJobHandler {
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        debug!(log_type = "starting", "{:?} job {} creation started", JobType::DataSubmission, internal_id);

        let job_item = JobItem::create(internal_id.clone(), JobType::DataSubmission, JobStatus::Created, metadata);

        debug!(log_type = "completed", "{:?} job {} creation completed", JobType::DataSubmission, internal_id);
        Ok(job_item)
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = &job.internal_id;
        info!(log_type = "starting", job_id = %job.id, "⚙️  {:?} job {} processing started", JobType::DataSubmission, internal_id);

        // Get DA-specific metadata
        let mut da_metadata: DaMetadata = job.metadata.specific.clone().try_into()?;
        let block_no = job.internal_id.parse::<u64>()?;

        let state_update = config
            .madara_rpc_client()
            .get_state_update(BlockId::Number(block_no))
            .await
            .map_err(|e| JobError::ProviderError(e.to_string()))?;

        let state_update = match state_update {
            MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
                warn!(block_no = block_no, "Block is still pending");
                Err(DaError::BlockPending { block_no: block_no.to_string(), job_id: job.id })?
            }
            MaybePreConfirmedStateUpdate::Update(state_update) => state_update,
        };
        debug!("Retrieved state update");

        // constructing the data from the rpc
        let blob_data = Self::state_update_to_blob_data(block_no, state_update, config.clone()).await?;
        // transforming the data so that we can apply FFT on this.
        let blob_data_biguint = Self::convert_to_biguint(blob_data.clone());
        debug!("Converted blob data to BigUint");

        let transformed_data = Self::fft_transformation(blob_data_biguint)
            .wrap_err("Failed to apply FFT transformation")
            .map_err(|e| {
                error!(error = ?e, "Failed to apply FFT transformation");
                JobError::Other(OtherError(e))
            })?;
        debug!("Applied FFT transformation");

        // Get blob data path from metadata
        let blob_data_path = da_metadata.blob_data_path.as_ref().ok_or_else(|| {
            error!("Blob data path not found in metadata");
            JobError::Other(OtherError(eyre!("Blob data path not found in metadata")))
        })?;

        // Store the transformed data
        Self::store_blob_data(transformed_data.clone(), blob_data_path, config.clone()).await?;
        debug!("Stored blob data");

        let max_bytes_per_blob = config.da_client().max_bytes_per_blob().await;
        let max_blob_per_txn = config.da_client().max_blob_per_txn().await;

        let blob_array = Self::data_to_blobs(max_bytes_per_blob, transformed_data)?;
        let current_blob_length: u64 = blob_array
            .len()
            .try_into()
            .wrap_err("Unable to convert the blob length into u64 format.".to_string())
            .map_err(|e| {
                error!(error = ?e, "Failed to convert blob length to u64");
                JobError::Other(OtherError(e))
            })?;
        debug!(blob_count = current_blob_length, "Converted data to blobs");

        // Check blob limit
        if current_blob_length > max_blob_per_txn {
            error!(
                job_id = ?job.id,
                current_blob_length = current_blob_length,
                max_blob_per_txn = max_blob_per_txn,
                "Exceeded maximum number of blobs per transaction"
            );
            Err(DaError::MaxBlobsLimitExceeded {
                max_blob_per_txn,
                current_blob_length,
                block_no: block_no.to_string(),
                job_id: job.id,
            })?
        }

        let settlement_start = Instant::now();

        let external_id = config.da_client().publish_state_diff(blob_array, &[0; 32]).await.map_err(|e| {
            error!(error = ?e, "Failed to publish state diff to DA layer");
            JobError::Other(OtherError(e))
        })?;

        // Record settlement time
        let settlement_duration = settlement_start.elapsed().as_secs_f64();
        MetricsRecorder::record_settlement_time(&job.job_type, settlement_duration);

        da_metadata.tx_hash = Some(external_id.clone());
        job.metadata.specific = JobSpecificMetadata::Da(da_metadata);

        info!(log_type = "completed", job_id = %job.id, external_id = ?external_id, "✅ {:?} job {} processed successfully", JobType::DataSubmission, internal_id);
        Ok(external_id)
    }

    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = &job.internal_id;
        debug!(log_type = "starting", job_id = %job.id, "{:?} job {} verification started", JobType::DataSubmission, internal_id);
        let verification_status = config
            .da_client()
            .verify_inclusion(job.external_id.unwrap_string().map_err(|e| {
                error!(error = ?e, "Failed to unwrap external ID");
                JobError::Other(OtherError(e))
            })?)
            .await
            .map_err(|e| {
                error!(job_id = ?job.id, error = ?e, "Job verification failed");
                JobError::Other(OtherError(e))
            })?
            .into();

        info!(log_type = "completed", job_id = %job.id, "🎯 {:?} job {} verification completed", JobType::DataSubmission, internal_id);
        Ok(verification_status)
    }
    fn max_process_attempts(&self, config: &Config) -> u64 {
        config.params.job_policies.da_submission.max_process_attempts
    }
    fn max_verification_attempts(&self, config: &Config) -> u64 {
        config.params.job_policies.da_submission.max_verification_attempts
    }
    fn verification_polling_delay_seconds(&self, config: &Config) -> u64 {
        config.params.job_policies.da_submission.verification_polling_delay_seconds
    }
}

#[cfg(test)]
pub mod test {
    use std::collections::HashSet;
    use std::fs;
    use std::fs::File;
    use std::io::Read;

    use crate::core::config::StarknetVersion;
    use crate::worker::event_handler::jobs::da::DAJobHandler;
    use ::serde::{Deserialize, Serialize};
    use color_eyre::Result;
    use httpmock::prelude::*;
    use majin_blob_core::blob;
    use majin_blob_types::serde;
    use orchestrator_da_client_interface::MockDaClient;
    use rstest::rstest;
    use serde_json::json;
    use starknet::core::types::{
        ContractStorageDiffItem, DeployedContractItem, Felt, NonceUpdate, StateDiff, StateUpdate, StorageEntry,
    };
    use starknet::providers::jsonrpc::HttpTransport;
    use starknet::providers::JsonRpcClient;
    use url::Url;

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
            .configure_madara_version(StarknetVersion::V0_13_2)
            .build()
            .await;

        get_nonce_attached(&server, nonce_file_path);

        let state_update = read_state_update_from_file(state_update_file_path).expect("issue while reading");
        let blob_data = DAJobHandler::state_update_to_blob_data(block_no, state_update, services.config)
            .await
            .expect("issue while converting state update to blob data");
        let blob_data_biguint = DAJobHandler::convert_to_biguint(blob_data);

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

        let original_blob_data = serde::parse_file_to_blob_data(file_to_check);
        // converting the data to its original format
        let ifft_blob_data = blob::recover(original_blob_data.clone());
        // applying the fft function again on the original format
        let fft_blob_data =
            DAJobHandler::fft_transformation(ifft_blob_data).expect("FFT transformation failed during test");

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

        DAJobHandler::refactor_state_update(&mut state_diff);

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
        if new_hex_chars.is_empty() {
            "0x0".to_string()
        } else {
            format!("0x{}", new_hex_chars)
        }
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
