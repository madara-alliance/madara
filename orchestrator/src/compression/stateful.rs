use crate::compression::utils::sort_state_diff;
use crate::utils::helpers::retry_async;
use color_eyre::{eyre, Result};
use futures::{stream, StreamExt};
use itertools::Itertools;
use starknet::core::types::{
    ContractStorageDiffItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem, StateUpdate, StorageEntry,
};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet_core::types::{BlockId, StarknetError};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

// https://community.starknet.io/t/starknet-v0-13-4-pre-release-notes/115257
const STATEFUL_SPECIAL_ADDRESS: Felt = Felt::from_hex_unchecked("0x2");
const STATEFUL_MAPPING_START: Felt = Felt::from_hex_unchecked("0x80"); // 128
const MAX_GET_STORAGE_AT_CALL_RETRY: u64 = 3;
const MAX_CONCURRENT_GET_STORAGE_AT_CALLS: usize = 3000;

/// Compresses a state update using stateful compression
///
/// This function:
/// 1. Extracts mapping information from the special address (0x2) and provider
/// 2. Use this mapping to transform other addresses and values
/// 3. Preserves all the entries in special addresses (<128) as it is
///
/// # Arguments
/// * `state_update` - StateUpdate to compress
/// * `last_block_before_state_update` - The last block before the state update
/// * `provider` - Provider to use for fetching values from the special address
///
/// # Returns
/// A compressed StateUpdate with values mapped according to the special address mappings
pub async fn compress(
    state_update: &StateUpdate,
    last_block_before_state_update: u64,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
) -> Result<StateUpdate> {
    let mut state_update = state_update.clone();

    let mapping =
        CompressedKeyValues::from_state_update_and_provider(&state_update, last_block_before_state_update, provider)
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
    /// Creates a new CompressedKeyValues using state update and provider
    /// This will create a HashMap which will contain all the required addresses and storage keys
    /// `key` - address/storage key
    /// `value` - compressed mapping for the address/storage key
    /// PLEASE NOTE: All the keys that will be required for the stateful compression will be present
    /// in the HashMap.
    /// You can read more about it in the community notes for stateful compression linked above
    async fn from_state_update_and_provider(
        state_update: &StateUpdate,
        last_block_before_state_update: u64,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
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

        // Fetch the values for the keys from the special address (0x2)
        let compressed_key_values = Self::get_compressed_key_values_from_state_update(state_update)?;

        // Fetch the value for the keys in the special address either from the current mapping or from the provider
        // Doing this in parallel
        stream::iter(keys)
            .map(|key| {
                let special_address_mappings = compressed_key_values.clone();
                async move {
                    match special_address_mappings.get(&key).cloned() {
                        Some(value) => Ok((key, value)),
                        None => Ok((
                            key,
                            Self::get_compressed_key_from_provider(provider, &key, last_block_before_state_update)
                                .await?,
                        )),
                    }
                }
            })
            .buffer_unordered(MAX_CONCURRENT_GET_STORAGE_AT_CALLS)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .try_collect::<_, Vec<(Felt, Felt)>, ProviderError>()?
            .iter()
            .for_each(|(key, value)| {
                mappings.insert(*key, *value);
            });
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

    /// Returns the mapping for a key after fetching it from the provider
    async fn get_compressed_key_from_provider(
        provider: &Arc<JsonRpcClient<HttpTransport>>,
        key: &Felt,
        last_block_before_state_update: u64,
    ) -> Result<Felt, ProviderError> {
        if Self::skip_address_compression(*key) {
            return Ok(*key);
        }

        retry_async(
            async || {
                provider
                    .get_storage_at(STATEFUL_SPECIAL_ADDRESS, key, BlockId::Number(last_block_before_state_update))
                    .await
            },
            MAX_GET_STORAGE_AT_CALL_RETRY,
            Some(Duration::from_secs(5)),
        )
        .await
        .map_err(|err| {
            ProviderError::StarknetError(StarknetError::UnexpectedError(format!(
                "Failed to get pre-range storage value for contract: {}, key: {} at block {}: {}",
                STATEFUL_SPECIAL_ADDRESS, key, last_block_before_state_update, err
            )))
        })
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
