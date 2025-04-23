use crate::core::config::Config;
use crate::error::event::EventSystemResult;
use crate::error::job::da_error::DaError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{DaMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::helpers::JobProcessingState;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::biguint_vec_to_u8_vec;
use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::{eyre, Context};
use lazy_static::lazy_static;
use num_bigint::{BigUint, ToBigUint};
use num_traits::{Num, Zero};
use starknet::providers::Provider;
use starknet_core::types::{
    BlockId, ContractStorageDiffItem, DeclaredClassItem, Felt, MaybePendingStateUpdate, StateDiff, StateUpdate,
};
use std::collections::{HashMap, HashSet};
use std::ops::{Add, Mul, Rem};
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

pub struct DAJobHandler;

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

    /// DA word encoding:
    /// |---padding---|---class flag---|---new nonce---|---num changes---|
    ///     127 bits        1 bit           64 bits          64 bits
    fn da_word(class_flag: bool, nonce_change: Option<Felt>, num_changes: u64) -> Result<Felt, JobError> {
        // padding of 127 bits
        let mut binary_string = "0".repeat(127);

        // class flag of one bit
        if class_flag {
            binary_string += "1"
        } else {
            binary_string += "0"
        }

        // checking for nonce here
        if let Some(new_nonce) = nonce_change {
            let bytes: [u8; 32] = new_nonce.to_bytes_be();
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

        let biguint = BigUint::from_str_radix(binary_string.as_str(), 2)
            .wrap_err("Failed to convert binary string to BigUint")
            .map_err(|e| JobError::Other(OtherError(e)))?;

        // Now convert the BigUint to a decimal string
        let decimal_string = biguint.to_str_radix(10);

        Felt::from_dec_str(&decimal_string)
            .wrap_err("Failed to convert decimal string to FieldElement")
            .map_err(|e| JobError::Other(OtherError(e)))
    }

    #[tracing::instrument(skip(elements))]
    pub fn fft_transformation(elements: Vec<BigUint>) -> Result<Vec<BigUint>, JobError> {
        let xs: Vec<BigUint> = (0..*BLOB_LEN)
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

            if nonce.is_none() && !storage_entries.is_empty() && address != Felt::ONE {
                let get_current_nonce_result = config
                    .starknet_client()
                    .get_nonce(BlockId::Number(block_no), address)
                    .await
                    .map_err(|e| JobError::ProviderError(format!("Failed to get nonce : {}", e)))?;

                nonce = Some(get_current_nonce_result);
            }
            let da_word = Self::da_word(class_flag.is_some(), nonce, storage_entries.len() as u64)?;
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
                tracing::debug!("Warning: Last chunk of {} bytes was padded to full blob size", chunk.len());
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
    #[tracing::instrument(fields(category = "da"), skip(self, metadata), ret, err)]
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        tracing::info!(log_type = "starting", category = "da", function_type = "create_job",  block_no = %internal_id, "DA job creation started.");

        let job_item = JobItem::create(internal_id.clone(), JobType::DataSubmission, JobStatus::Created, metadata);

        tracing::info!(log_type = "completed", category = "da", function_type = "create_job", block_no = %internal_id, "DA job creation completed.");
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "da"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "da",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "DA job processing started."
        );

        // Get DA-specific metadata
        let mut da_metadata: DaMetadata = job.metadata.specific.clone().try_into()?;
        let block_no = job.internal_id.parse::<u64>()?;

        let state_update = config
            .starknet_client()
            .get_state_update(BlockId::Number(block_no))
            .await
            .map_err(|e| JobError::ProviderError(e.to_string()))?;

        let state_update = match state_update {
            MaybePendingStateUpdate::PendingUpdate(_) => {
                tracing::warn!(job_id = ?job.id, block_no = block_no, "Block is still pending");
                Err(DaError::BlockPending { block_no: block_no.to_string(), job_id: job.id })?
            }
            MaybePendingStateUpdate::Update(state_update) => state_update,
        };
        tracing::debug!(job_id = ?job.id, "Retrieved state update");

        // constructing the data from the rpc
        let blob_data = Self::state_update_to_blob_data(block_no, state_update, config.clone()).await?;
        // transforming the data so that we can apply FFT on this.
        let blob_data_biguint = Self::convert_to_biguint(blob_data.clone());
        tracing::trace!(job_id = ?job.id, "Converted blob data to BigUint");

        let transformed_data = Self::fft_transformation(blob_data_biguint)
            .wrap_err("Failed to apply FFT transformation")
            .map_err(|e| {
                tracing::error!(job_id = ?job.id, error = ?e, "Failed to apply FFT transformation");
                JobError::Other(OtherError(e))
            })?;
        tracing::trace!(job_id = ?job.id, "Applied FFT transformation");

        // Get blob data path from metadata
        let blob_data_path = da_metadata.blob_data_path.as_ref().ok_or_else(|| {
            tracing::error!(job_id = ?job.id, "Blob data path not found in metadata");
            JobError::Other(OtherError(eyre!("Blob data path not found in metadata")))
        })?;

        // Store the transformed data
        Self::store_blob_data(transformed_data.clone(), blob_data_path, config.clone()).await?;
        tracing::debug!(job_id = ?job.id, "Stored blob data");

        let max_bytes_per_blob = config.da_client().max_bytes_per_blob().await;
        let max_blob_per_txn = config.da_client().max_blob_per_txn().await;
        tracing::trace!(
            job_id = ?job.id,
            max_bytes_per_blob = max_bytes_per_blob,
            max_blob_per_txn = max_blob_per_txn,
            "Retrieved DA client configuration"
        );

        let blob_array = Self::data_to_blobs(max_bytes_per_blob, transformed_data)?;
        let current_blob_length: u64 = blob_array
            .len()
            .try_into()
            .wrap_err("Unable to convert the blob length into u64 format.".to_string())
            .map_err(|e| {
                tracing::error!(job_id = ?job.id, error = ?e, "Failed to convert blob length to u64");
                JobError::Other(OtherError(e))
            })?;
        tracing::debug!(job_id = ?job.id, blob_count = current_blob_length, "Converted data to blobs");

        // Check blob limit
        if current_blob_length > max_blob_per_txn {
            tracing::error!(
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

        // Publish to DA layer
        let external_id = config.da_client().publish_state_diff(blob_array, &[0; 32]).await.map_err(|e| {
            tracing::error!(job_id = ?job.id, error = ?e, "Failed to publish state diff to DA layer");
            JobError::Other(OtherError(e))
        })?;

        da_metadata.tx_hash = Some(external_id.clone());
        job.metadata.specific = JobSpecificMetadata::Da(da_metadata);

        tracing::info!(
            log_type = "completed",
            category = "da",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            external_id = ?external_id,
            "Successfully published state diff to DA layer."
        );
        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "da"), skip(self, config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting", category = "da",
            function_type = "verify_job", job_id = ?job.id,
            block_no = %internal_id, "DA job verification started."
        );
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
    fn job_processing_lock(&self, _config: Arc<Config>) -> Option<Arc<JobProcessingState>> {
        None
    }
}
