use color_eyre::{eyre, Result};
use futures::{stream, StreamExt};
use starknet::core::types::{
    ContractStorageDiffItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem, StateUpdate, StorageEntry,
};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError};
use starknet_core::types::{BlockId, DeclaredClassItem, StarknetError};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const SPECIAL_ADDRESS: &str = "0x2";
const MAPPING_START: &str = "0x80";  // 128

/// Represents a mapping from one value to another
#[derive(Debug)]
struct ValueMapping {
    mappings: HashMap<Felt, Felt>,
}

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
        state_update.state_diff.storage_diffs.iter().for_each(|diff| {
            // Skip the special address
            if ValueMapping::skip(diff.address) {
                return;
            }
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

        // Fetch the values for the keys from the special address (0x2) or the provider
        let special_address_mappings = ValueMapping::get_special_address_mappings(state_update)?;

        let fetch_results: Vec<_> = stream::iter(keys)
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
            .collect()
            .await;

        for (key, value) in parse_results(fetch_results)? {
            mappings.insert(key, value);
        }

        Ok(ValueMapping { mappings })
    }

    /// get_special_address_mappings create a hashmap from the storage mappings at the special address
    fn get_special_address_mappings(state_update: &StateUpdate) -> Result<HashMap<Felt, Felt>> {
        // Find the special address storage entries
        if let Some(special_contract) = state_update
            .state_diff
            .storage_diffs
            .iter()
            .find(|diff| diff.address == Felt::from_hex(SPECIAL_ADDRESS).unwrap())
        {
            let mut mappings: HashMap<Felt, Felt> = HashMap::new();

            // Add each key-value pair to our mapping, ignoring the global counter-slot
            for entry in &special_contract.storage_entries {
                if !ValueMapping::skip(entry.key) {
                    mappings.insert(entry.key, entry.value);
                }
            }

            Ok(mappings)
        } else {
            Err(eyre::eyre!("Special address not found in state update"))
        }
    }

    /// skip determines if we can skip a contract or storage address from stateful compression mapping
    fn skip(address: Felt) -> bool {
        address < Felt::from_hex(MAPPING_START).unwrap()
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
        match provider
            .get_storage_at(Felt::from_hex(SPECIAL_ADDRESS).unwrap(), key, BlockId::Number(pre_range_block))
            .await
        {
            Ok(value) => Ok(value),
            Err(e) => Err(ProviderError::StarknetError(StarknetError::UnexpectedError(format!(
                "Failed to get pre-range storage value for contract: {}, key: {} at block {}: {}",
                SPECIAL_ADDRESS, key, pre_range_block, e
            )))),
        }
    }

    fn get_value(&self, value: &Felt) -> Result<Felt> {
        match self.mappings.get(value).cloned() {
            Some(value) => Ok(value),
            None => Err(eyre::eyre!("Value not found in mapping: {}", value)),
        }
    }
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
    let mut new_storage_diffs = Vec::new();
    for diff in state_update.state_diff.storage_diffs {
        if ValueMapping::skip(diff.address) {
            new_storage_diffs.push(diff);
            continue;
        }

        let mapped_address = mapping.get_value(&diff.address)?;
        let mut mapped_entries = Vec::new();

        for entry in diff.storage_entries {
            mapped_entries.push(StorageEntry { key: mapping.get_value(&entry.key)?, value: entry.value });
        }

        new_storage_diffs.push(ContractStorageDiffItem { address: mapped_address, storage_entries: mapped_entries });
    }
    state_update.state_diff.storage_diffs = new_storage_diffs;

    // Process deployed contracts
    let mut new_deployed_contracts = Vec::new();
    for item in state_update.state_diff.deployed_contracts {
        new_deployed_contracts
            .push(DeployedContractItem { address: mapping.get_value(&item.address)?, class_hash: item.class_hash });
    }
    state_update.state_diff.deployed_contracts = new_deployed_contracts;

    // Declared class remain as it is as it only contains class hashes

    // Process nonces
    let mut new_nonces = Vec::new();
    for item in state_update.state_diff.nonces {
        new_nonces
            .push(NonceUpdate { contract_address: mapping.get_value(&item.contract_address)?, nonce: item.nonce });
    }
    state_update.state_diff.nonces = new_nonces;

    // Process replaced classes
    let mut new_replaced_classes = Vec::new();
    for item in state_update.state_diff.replaced_classes {
        new_replaced_classes.push(ReplacedClassItem {
            contract_address: mapping.get_value(&item.contract_address)?,
            class_hash: item.class_hash,
        });
    }
    state_update.state_diff.replaced_classes = new_replaced_classes;

    // Deprecated declared classes remain as it is as it only contains class hashes
    // block_hash, new_root and old_root remain as it is

    Ok(state_update)
}

fn parse_results<T>(results: Vec<Result<T>>) -> Result<Vec<T>> {
    let mut values = Vec::new();
    for result in results {
        match result {
            Ok(value) => {
                values.push(value);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(values)
}
