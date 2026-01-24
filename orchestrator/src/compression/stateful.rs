use crate::compression::batch_rpc::BatchRpcClient;
use crate::compression::utils::sort_state_diff;
use color_eyre::{eyre, Result};
use starknet::core::types::{
    ContractStorageDiffItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem, StateUpdate, StorageEntry,
};
use starknet_core::types::BlockId;
use std::collections::{HashMap, HashSet};
use tracing::{debug, warn};

// https://community.starknet.io/t/starknet-v0-13-4-pre-release-notes/115257
const STATEFUL_SPECIAL_ADDRESS: Felt = Felt::from_hex_unchecked("0x2");
const STATEFUL_MAPPING_START: Felt = Felt::from_hex_unchecked("0x80"); // 128

/// Compresses a state update using stateful compression
///
/// This function:
/// 1. Extracts mapping information from the special address (0x2) and batch RPC client
/// 2. Use this mapping to transform other addresses and values
/// 3. Preserves all the entries in special addresses (<128) as it is
///
/// # Arguments
/// * `state_update` - StateUpdate to compress
/// * `last_block_before_state_update` - The last block before the state update
/// * `batch_client` - Batch RPC client for efficient batch queries
///
/// # Returns
/// A compressed StateUpdate with values mapped according to the special address mappings
pub async fn compress(
    state_update: &StateUpdate,
    last_block_before_state_update: u64,
    batch_client: &BatchRpcClient,
) -> Result<StateUpdate> {
    let mut state_update = state_update.clone();

    let mapping =
        CompressedKeyValues::from_state_update_batched(&state_update, last_block_before_state_update, batch_client)
            .await?;

    // Process storage diffs
    state_update.state_diff.storage_diffs = process_storage_diffs(state_update.state_diff.storage_diffs, &mapping)?;

    // Process deployed contracts
    state_update.state_diff.deployed_contracts =
        process_deployed_contracts(state_update.state_diff.deployed_contracts, &mapping)?;

    // Process nonces
    state_update.state_diff.nonces = process_nonces(state_update.state_diff.nonces, &mapping)?;

    // Process replaced classes
    state_update.state_diff.replaced_classes =
        process_replaced_classes(state_update.state_diff.replaced_classes, &mapping)?;

    // Declared class remain as it is as it only contains class hashes
    // Deprecated declared classes remain as it is as it only contains class hashes
    // block_hash, new_root and old_root remain as it is

    // Sort the compressed StateUpdate
    sort_state_diff(&mut state_update);
    Ok(state_update)
}

/// Represents a mapping from one value to another
#[derive(Debug)]
struct CompressedKeyValues(HashMap<Felt, Felt>);

impl CompressedKeyValues {
    /// Creates a new CompressedKeyValues using state update and batch RPC client
    /// This will create a HashMap which will contain all the required addresses and storage keys
    /// `key` - address/storage key
    /// `value` - compressed mapping for the address/storage key
    /// PLEASE NOTE: All the keys that will be required for the stateful compression will be present
    /// in the HashMap.
    /// You can read more about it in the community notes for stateful compression linked above
    async fn from_state_update_batched(
        state_update: &StateUpdate,
        last_block_before_state_update: u64,
        batch_client: &BatchRpcClient,
    ) -> Result<Self> {
        let mut mappings: HashMap<Felt, Felt> = HashMap::new();
        let mut keys: HashSet<Felt> = HashSet::new();

        // Collecting all the keys for which mapping might be required
        state_update
            .state_diff
            .storage_diffs
            .iter()
            .filter(|diff| !Self::skip_address_compression(diff.address))
            .for_each(|diff| {
                keys.insert(diff.address);
                diff.storage_entries.iter().for_each(|entry| {
                    keys.insert(entry.key);
                });
            });
        state_update.state_diff.deployed_contracts.iter().for_each(|contract| {
            keys.insert(contract.address);
        });
        state_update.state_diff.nonces.iter().for_each(|nonce| {
            keys.insert(nonce.contract_address);
        });
        state_update.state_diff.replaced_classes.iter().for_each(|replaced_class| {
            keys.insert(replaced_class.contract_address);
        });

        // Fetch the values for the keys from the special address (0x2) in the current state update
        let compressed_key_values = Self::get_compressed_key_values_from_state_update(state_update)?;

        // Separate keys into those found in state update vs those needing provider lookup
        let mut keys_needing_provider: Vec<Felt> = Vec::new();

        for key in &keys {
            if Self::skip_address_compression(*key) {
                // Keys below threshold map to themselves
                mappings.insert(*key, *key);
            } else if let Some(value) = compressed_key_values.get(key) {
                // Found in current state update
                mappings.insert(*key, *value);
            } else {
                // Need to fetch from provider
                keys_needing_provider.push(*key);
            }
        }

        // Batch-fetch all keys that need provider lookup
        if !keys_needing_provider.is_empty() {
            debug!(
                "Batch-fetching {} compressed key values from provider at block {}",
                keys_needing_provider.len(),
                last_block_before_state_update
            );

            // Build storage queries: all keys are fetched from STATEFUL_SPECIAL_ADDRESS
            let storage_queries: Vec<(Felt, Felt)> =
                keys_needing_provider.iter().map(|key| (STATEFUL_SPECIAL_ADDRESS, *key)).collect();

            let block_id = BlockId::Number(last_block_before_state_update);
            let provider_values = batch_client
                .batch_get_storage_at(storage_queries, block_id)
                .await
                .map_err(|e| eyre::eyre!("Failed to batch get compressed key values from provider: {}", e))?;

            // Add fetched values to mappings
            for key in keys_needing_provider {
                let value = provider_values.get(&(STATEFUL_SPECIAL_ADDRESS, key)).copied().unwrap_or(Felt::ZERO);
                mappings.insert(key, value);
            }
        }

        Ok(Self(mappings))
    }

    /// Creates a hashmap from the storage mappings at the special address
    fn get_compressed_key_values_from_state_update(state_update: &StateUpdate) -> Result<HashMap<Felt, Felt>> {
        // Find the special address storage entries
        let mut mappings: HashMap<Felt, Felt> = HashMap::new();
        if let Some(special_contract) =
            state_update.state_diff.storage_diffs.iter().find(|diff| diff.address == STATEFUL_SPECIAL_ADDRESS)
        {
            // Add each key-value pair to our mapping, ignoring the global counter-slot
            special_contract
                .storage_entries
                .iter()
                .filter(|entry| !Self::skip_address_compression(entry.key))
                .for_each(|entry| {
                    mappings.insert(entry.key, entry.value);
                });
        } else {
            warn!("didn't get any key for the alias address of 0x2");
        }
        Ok(mappings)
    }

    /// Determines if we can skip a contract or storage address from stateful compression mapping
    fn skip_address_compression(address: Felt) -> bool {
        address < STATEFUL_MAPPING_START
    }

    /// Returns the compressed key for a given value
    fn get_compressed_value(&self, key: &Felt) -> Result<Felt> {
        self.0.get(key).cloned().ok_or(eyre::eyre!("Compressed value not found in mapping for key {}", key))
    }
}

fn process_storage_diffs(
    storage_diffs: Vec<ContractStorageDiffItem>,
    mapping: &CompressedKeyValues,
) -> Result<Vec<ContractStorageDiffItem>> {
    let mut new_storage_diffs: Vec<ContractStorageDiffItem> = Vec::new();
    for diff in storage_diffs {
        if CompressedKeyValues::skip_address_compression(diff.address) {
            new_storage_diffs.push(diff);
            continue;
        }

        let mapped_address = mapping.get_compressed_value(&diff.address)?;
        let mut mapped_entries = Vec::new();

        for entry in diff.storage_entries {
            mapped_entries.push(StorageEntry { key: mapping.get_compressed_value(&entry.key)?, value: entry.value });
        }

        new_storage_diffs.push(ContractStorageDiffItem { address: mapped_address, storage_entries: mapped_entries });
    }
    Ok(new_storage_diffs)
}

fn process_deployed_contracts(
    deployed_contracts: Vec<DeployedContractItem>,
    mapping: &CompressedKeyValues,
) -> Result<Vec<DeployedContractItem>> {
    deployed_contracts
        .into_iter()
        .map(|item| {
            Ok(DeployedContractItem {
                address: mapping.get_compressed_value(&item.address)?,
                class_hash: item.class_hash,
            })
        })
        .collect()
}

fn process_nonces(nonces: Vec<NonceUpdate>, mapping: &CompressedKeyValues) -> Result<Vec<NonceUpdate>> {
    nonces
        .into_iter()
        .map(|item| {
            Ok(NonceUpdate {
                contract_address: mapping.get_compressed_value(&item.contract_address)?,
                nonce: item.nonce,
            })
        })
        .collect()
}

fn process_replaced_classes(
    replaced_classes: Vec<ReplacedClassItem>,
    mapping: &CompressedKeyValues,
) -> Result<Vec<ReplacedClassItem>> {
    replaced_classes
        .into_iter()
        .map(|item| {
            Ok(ReplacedClassItem {
                contract_address: mapping.get_compressed_value(&item.contract_address)?,
                class_hash: item.class_hash,
            })
        })
        .collect()
}
