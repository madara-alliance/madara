use crate::compression::blob::da_word;
use crate::core::config::StarknetVersion;
use crate::error::job::JobError;
use crate::types::constant::BLOB_LEN;
use itertools::repeat_n;
use num_bigint::BigUint;
use num_traits::Zero;
use starknet_core::types::{ContractStorageDiffItem, DeclaredClassItem, Felt, StorageEntry};
use std::collections::{HashMap, HashSet};

pub(super) struct BlobContractDiff {
    pub(super) address: Felt,
    pub(super) da_word: Felt,
    pub(super) class_hash: Option<Felt>,
    pub(super) storage_entries: Vec<StorageEntry>,
}

pub(super) struct BlobContractDiffVec(Vec<BlobContractDiff>);

impl BlobContractDiffVec {
    /// Create a vector of BlobContractDiff from a vector of ContractStorageDiffItem
    /// In the process, it also removes the used values from
    /// - processed_addresses
    /// - deployed_contracts
    /// - replaced_classes
    /// - nonces
    ///
    /// These need to be passed as mutable reference to the function
    pub(super) fn from_storage_diffs(
        storage_diffs: Vec<ContractStorageDiffItem>,
        processed_addresses: &mut HashSet<Felt>,
        deployed_contracts: &mut HashMap<Felt, Felt>,
        replaced_classes: &mut HashMap<Felt, Felt>,
        nonces: &mut HashMap<Felt, Felt>,
        version: StarknetVersion,
    ) -> Result<Self, JobError> {
        let mut contract_diffs = Vec::new();

        for ContractStorageDiffItem { address, storage_entries } in storage_diffs.into_iter() {
            tracing::debug!("Processing storage diffs for address {}", address);
            // Mark this address as processed
            processed_addresses.insert(address);

            // Get class hash from deployed_contracts or replaced_classes
            let mut class_hash = None;

            // First, try deployed contracts
            if let Some(hash) = deployed_contracts.remove(&address) {
                tracing::debug!("Found deployed contract at address {} with class_hash {}", address, hash);
                class_hash = Some(hash);
            }
            // Then try replaced classes
            else if let Some(hash) = replaced_classes.remove(&address) {
                tracing::debug!("Found replaced class at address {} with class_hash {}", address, hash);
                class_hash = Some(hash);
            }

            // Get nonce if it exists and remove from the map since we're processing this address
            let nonce = nonces.remove(&address);

            // Create the DA word - class_flag is true if class_hash is Some (i.e., the class is replaced)
            let da_word = da_word(class_hash.is_some(), nonce, storage_entries.len() as u64, version)?;

            contract_diffs.push(BlobContractDiff { address, da_word, class_hash, storage_entries })
        }

        Ok(BlobContractDiffVec(contract_diffs))
    }

    pub(super) fn add_addresses_without_storage_diffs(
        &mut self,
        processed_addresses: &mut HashSet<Felt>,
        deployed_contracts: &mut HashMap<Felt, Felt>,
        replaced_classes: &mut HashMap<Felt, Felt>,
        nonces: &mut HashMap<Felt, Felt>,
        version: StarknetVersion,
    ) -> Result<usize, JobError> {
        let addresses = Self::collect_addresses_without_storage_diffs(
            processed_addresses,
            deployed_contracts,
            replaced_classes,
            nonces,
        );
        tracing::debug!(
            "Processing {} leftover addresses with nonce or class updates but no storage updates",
            addresses.len()
        );
        for (address, class_hash, nonce) in addresses.iter() {
            self.0.push(BlobContractDiff {
                address: *address,
                da_word: da_word(class_hash.is_some(), *nonce, 0, version)?,
                class_hash: *class_hash,
                storage_entries: Vec::new(),
            })
        }
        Ok(addresses.len())
    }

    pub(super) fn sort_contracts(&mut self) {
        self.0.sort_by_key(|contract_diff| contract_diff.address);
    }

    /// Collects leftover addresses (nonce/class updates without storage) and returns a sorted vector
    fn collect_addresses_without_storage_diffs(
        processed_addresses: &HashSet<Felt>,
        deployed_contracts: &mut HashMap<Felt, Felt>,
        replaced_classes: &mut HashMap<Felt, Felt>,
        nonces: &mut HashMap<Felt, Felt>,
    ) -> Vec<(Felt, Option<Felt>, Option<Felt>)> {
        let mut leftover_addresses = Vec::new();
        leftover_addresses.extend(
            deployed_contracts
                .drain()
                .filter(|(address, _)| !processed_addresses.contains(address))
                .map(|(address, class_hash)| (address, Some(class_hash), nonces.remove(&address))),
        );
        leftover_addresses.extend(
            replaced_classes
                .drain()
                .filter(|(address, _)| !processed_addresses.contains(address))
                .map(|(address, class_hash)| (address, Some(class_hash), nonces.remove(&address))),
        );
        leftover_addresses.extend(
            nonces
                .drain()
                .filter(|(address, _)| !processed_addresses.contains(address))
                .map(|(address, nonce)| (address, None, Some(nonce))),
        );
        leftover_addresses.sort_by_key(|(address, _, _)| *address);
        leftover_addresses
    }
}

/// Appends declared classes count and their data to blob_data
pub(super) fn add_declared_classes_to_blob_data(
    mut declared_classes: Vec<DeclaredClassItem>,
    blob_data: &mut Vec<Felt>,
) {
    // Sorting declared classes by class hash for deterministic output
    declared_classes.sort_by_key(|class| class.class_hash);

    tracing::debug!("Total declared classes in blob: {}", declared_classes.len());

    blob_data.push(Felt::from(declared_classes.len()));
    for DeclaredClassItem { class_hash, compiled_class_hash } in declared_classes.into_iter() {
        blob_data.push(class_hash);
        blob_data.push(compiled_class_hash);
    }
}

/// Adds all the contract diffs to blob_data
pub(super) fn add_contract_diffs_to_blob_data(
    contract_diffs: BlobContractDiffVec,
    blob_data: &mut Vec<Felt>,
) -> Result<(), JobError> {
    for contract_diff in contract_diffs.0.into_iter() {
        blob_data.push(contract_diff.address);
        blob_data.push(contract_diff.da_word);
        if let Some(class_hash) = contract_diff.class_hash {
            blob_data.push(class_hash);
        }
        for storage_entry in contract_diff.storage_entries.into_iter() {
            blob_data.push(storage_entry.key);
            blob_data.push(storage_entry.value);
        }
    }

    Ok(())
}

/// Converts a vector of felt into a vector of bigUint
/// The output length depends on the input length (ceil(input_len / BLOB_LEN) * BLOB_LEN)
pub(super) fn convert_to_biguint(elements: &[Felt]) -> Vec<BigUint> {
    let num_blocks = elements.len().div_ceil(BLOB_LEN);
    let output_len = num_blocks * BLOB_LEN;
    let pad_len = output_len - elements.len();

    elements
        .iter()
        .map(|e| e.to_biguint())
        .chain(repeat_n(BigUint::zero(), pad_len))
        .collect()
}
