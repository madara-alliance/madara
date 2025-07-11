use crate::error::job::JobError;
use color_eyre::{eyre, Report, Result};
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
use tracing::log::error;

const SPECIAL_ADDRESS: Felt = Felt::from_hex_unchecked("0x2");
const MAPPING_START: Felt = Felt::from_hex_unchecked("0x80"); // 128
const MAX_GET_STORAGE_AT_CALL_RETRY: u64 = 3;

/// Represents a mapping from one value to another
#[derive(Debug)]
struct ValueMapping(HashMap<Felt, Felt>);

impl ValueMapping {
    /// Creates a new ValueMapping using state update and provider
    async fn from_state_update_or_provider(
        state_update: &StateUpdate,
        pre_range_block: u64,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<Self> {
        let mut mappings: HashMap<Felt, Felt> = HashMap::new();
        let mut keys: HashSet<Felt> = HashSet::new();

        // Collecting all the keys for which mapping might be required
        state_update.state_diff.storage_diffs.iter().filter(|diff| ValueMapping::skip(diff.address)).for_each(|diff| {
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
        let special_address_mappings = ValueMapping::get_special_address_mappings(state_update)?;

        // Fetch the value for the keys in the special address either from the current mapping or from the provider
        // Doing this in parallel
        stream::iter(keys)
            .map(|key| {
                let special_address_mappings = special_address_mappings.clone();
                async move {
                    match special_address_mappings.get(&key).cloned() {
                        Some(value) => Ok((key, value)),
                        None => {
                            Ok((key, ValueMapping::get_value_from_provider(provider, &key, pre_range_block).await?))
                        }
                    }
                }
            })
            .buffer_unordered(3000)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .try_collect::<_, Vec<(Felt, Felt)>, ProviderError>()?
            .iter()
            .for_each(|(key, value)| {
                mappings.insert(*key, *value);
            });

        Ok(ValueMapping(mappings))
    }

    /// get_special_address_mappings create a hashmap from the storage mappings at the special address
    fn get_special_address_mappings(state_update: &StateUpdate) -> Result<HashMap<Felt, Felt>> {
        // Find the special address storage entries
        if let Some(special_contract) =
            state_update.state_diff.storage_diffs.iter().find(|diff| diff.address == SPECIAL_ADDRESS)
        {
            let mut mappings: HashMap<Felt, Felt> = HashMap::new();

            // Add each key-value pair to our mapping, ignoring the global counter-slot
            special_contract.storage_entries.iter().filter(|entry| !ValueMapping::skip(entry.key)).for_each(|entry| {
                mappings.insert(entry.key, entry.value);
            });

            Ok(mappings)
        } else {
            Err(eyre::eyre!("Special address not found in state update"))
        }
    }

    /// skip determines if we can skip a contract or storage address from stateful compression mapping
    fn skip(address: Felt) -> bool {
        address < MAPPING_START
    }

    /// get_value_from_provider returns the mapping for a key after fetching it from the provider
    async fn get_value_from_provider(
        provider: &Arc<JsonRpcClient<HttpTransport>>,
        key: &Felt,
        pre_range_block: u64,
    ) -> Result<Felt, ProviderError> {
        if ValueMapping::skip(*key) {
            return Ok(key.clone());
        }
        let mut attempts = 0;
        let mut error = String::from("Dummy Error");
        while attempts < MAX_GET_STORAGE_AT_CALL_RETRY {
            match provider.get_storage_at(MAPPING_START, key, BlockId::Number(pre_range_block)).await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    error = e.to_string();
                    attempts += 1;
                    continue;
                }
            }
        }
        let err_message = format!(
            "Failed to get pre-range storage value for contract: {}, key: {} at block {}: {}",
            SPECIAL_ADDRESS, key, pre_range_block, error
        );
        error!("{}", &err_message);
        Err(ProviderError::StarknetError(StarknetError::UnexpectedError(err_message)))
    }

    fn get_value(&self, value: &Felt) -> Result<Felt> {
        self.0.get(value).cloned().ok_or(eyre::eyre!("Value not found in mapping: {}", value))
    }
}

fn process_storage_diffs(state_update: &StateUpdate, mapping: &ValueMapping) -> Result<Vec<ContractStorageDiffItem>> {
    let mut new_storage_diffs: Vec<ContractStorageDiffItem> = Vec::new();
    for diff in &state_update.state_diff.storage_diffs {
        if ValueMapping::skip(diff.address) {
            new_storage_diffs.push(diff.clone());
            continue;
        }

        let mapped_address = mapping.get_value(&diff.address)?;
        let mut mapped_entries = Vec::new();

        for entry in &diff.storage_entries {
            mapped_entries.push(StorageEntry { key: mapping.get_value(&entry.key)?, value: entry.value });
        }

        new_storage_diffs.push(ContractStorageDiffItem { address: mapped_address, storage_entries: mapped_entries });
    }
    Ok(new_storage_diffs)
}

fn process_deployed_contracts(state_update: &StateUpdate, mapping: &ValueMapping) -> Result<Vec<DeployedContractItem>> {
    let mut new_deployed_contracts = Vec::new();
    for item in &state_update.state_diff.deployed_contracts {
        new_deployed_contracts
            .push(DeployedContractItem { address: mapping.get_value(&item.address)?, class_hash: item.class_hash });
    }
    Ok(new_deployed_contracts)
}

fn process_nonces(state_update: &StateUpdate, mapping: &ValueMapping) -> Result<Vec<NonceUpdate>> {
    let mut new_nonces = Vec::new();
    for item in &state_update.state_diff.nonces {
        new_nonces
            .push(NonceUpdate { contract_address: mapping.get_value(&item.contract_address)?, nonce: item.nonce });
    }
    Ok(new_nonces)
}

fn process_replaced_classes(state_update: &StateUpdate, mapping: &ValueMapping) -> Result<Vec<ReplacedClassItem>> {
    let mut new_replaced_classes = Vec::new();
    for item in &state_update.state_diff.replaced_classes {
        new_replaced_classes.push(ReplacedClassItem {
            contract_address: mapping.get_value(&item.contract_address)?,
            class_hash: item.class_hash,
        });
    }
    Ok(new_replaced_classes)
}

/// Compresses a state update using stateful compression
///
/// This function:
/// 1. Extracts mapping information from the special address (0x2) and provider
/// 2. Use this mapping to transform other addresses and values
/// 3. Preserves all the entries in special addresses (<128) as it is
///
/// # Arguments
/// * `state_update` - StateUpdate to compress
///
/// # Returns
/// A compressed StateUpdate with values mapped according to the special address mappings
pub async fn compress(
    state_update: &StateUpdate,
    pre_range_block: u64,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
) -> Result<StateUpdate> {
    let mut state_update = state_update.clone();

    let mapping = ValueMapping::from_state_update_or_provider(&state_update, pre_range_block, provider).await?;

    // Process storage diffs
    state_update.state_diff.storage_diffs = process_storage_diffs(&state_update, &mapping)?;

    // Process deployed contracts
    state_update.state_diff.deployed_contracts = process_deployed_contracts(&state_update, &mapping)?;

    // Process nonces
    state_update.state_diff.nonces = process_nonces(&state_update, &mapping)?;

    // Process replaced classes
    state_update.state_diff.replaced_classes = process_replaced_classes(&state_update, &mapping)?;

    // Declared class remain as it is as it only contains class hashes
    // Deprecated declared classes remain as it is as it only contains class hashes
    // block_hash, new_root and old_root remain as it is

    Ok(state_update)
}
