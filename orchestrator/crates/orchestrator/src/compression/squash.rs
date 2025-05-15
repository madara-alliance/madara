use crate::error::job::JobError;
use crate::error::other::OtherError;
use color_eyre::eyre::eyre;
use futures::future::try_join_all;
use futures::stream;
use futures::stream::StreamExt;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet_core::types::{
    BlockId, ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem,
    StateDiff, StateUpdate, StorageEntry,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const MAX_CONCURRENT_CONTRACTS_PROCESSING: usize = 100;

/// squash_state_updates merge all the StateUpdate into a single StateUpdate
pub async fn squash_state_updates(
    state_updates: Vec<StateUpdate>,
    pre_range_block: u64,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
) -> Result<StateUpdate, JobError> {
    if state_updates.is_empty() {
        return Err(JobError::Other(OtherError(eyre!("Cannot merge empty state updates"))));
    }

    // Take the last block hash and number from the last update as our "latest"
    let last_update = state_updates.last().ok_or(JobError::Other(OtherError(eyre!("Invalid state updates"))))?;
    let block_hash = last_update.block_hash;
    let new_root = last_update.new_root;
    let old_root = state_updates.first().ok_or(JobError::Other(OtherError(eyre!("Invalid state updates"))))?.old_root;

    // Create a new StateDiff to hold the merged state
    let mut state_diff = StateDiff {
        storage_diffs: Vec::new(),
        deployed_contracts: Vec::new(),
        declared_classes: Vec::new(),
        deprecated_declared_classes: Vec::new(),
        nonces: Vec::new(),
        replaced_classes: Vec::new(),
    };

    // Maps to efficiently track the latest state
    let mut storage_diffs_map: HashMap<Felt, HashMap<Felt, Felt>> = HashMap::new();
    let mut deployed_contracts_map: HashMap<Felt, Felt> = HashMap::new();
    let mut declared_classes_map: HashMap<Felt, Felt> = HashMap::new();
    let mut nonces_map: HashMap<Felt, Felt> = HashMap::new();
    let mut replaced_classes_map: HashMap<Felt, Felt> = HashMap::new();
    let mut deprecated_classes_set: HashSet<Felt> = HashSet::new();

    // Process each update in order
    for update in state_updates {
        // Process storage diffs
        for contract_diff in update.state_diff.storage_diffs {
            let contract_addr = contract_diff.address;
            let contract_storage_map = storage_diffs_map.entry(contract_addr).or_default();

            for entry in contract_diff.storage_entries {
                contract_storage_map.insert(entry.key, entry.value);
            }
        }

        // Process deployed contracts
        for item in update.state_diff.deployed_contracts {
            deployed_contracts_map.insert(item.address, item.class_hash);
        }

        // Process declared classes
        for item in update.state_diff.declared_classes {
            declared_classes_map.insert(item.class_hash, item.compiled_class_hash);
        }

        // Process nonces
        for item in update.state_diff.nonces {
            nonces_map.insert(item.contract_address, item.nonce);
        }

        // Process replaced classes
        for item in update.state_diff.replaced_classes {
            replaced_classes_map.insert(item.contract_address, item.class_hash);
        }

        // Process deprecated classes
        for class_hash in update.state_diff.deprecated_declared_classes {
            deprecated_classes_set.insert(class_hash);
        }
    }

    // Convert maps back to the required StateDiff format
    let mut no_of_contracts = 0;

    let results: Vec<_> = stream::iter(storage_diffs_map)
        .map(|(contract_addr, storage_map)| async move {
            process_single_contract(contract_addr, storage_map, provider, pre_range_block).await
        })
        .buffer_unordered(MAX_CONCURRENT_CONTRACTS_PROCESSING)
        .collect()
        .await;

    for result in results {
        match result {
            Ok(Some(contract_storage_diff)) => {
                state_diff.storage_diffs.push(contract_storage_diff);
            }
            Ok(None) => {} // No storage entries for this contract
            Err(e) => return Err(e),
        }
        no_of_contracts += 1;
    }

    // Deployed contracts
    state_diff.deployed_contracts = deployed_contracts_map
        .into_iter()
        .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
        .collect();

    // Declared classes
    state_diff.declared_classes = declared_classes_map
        .into_iter()
        .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
        .collect();

    // Nonces
    state_diff.nonces =
        nonces_map.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect();

    // Replaced classes
    state_diff.replaced_classes = replaced_classes_map
        .into_iter()
        .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
        .collect();

    // Deprecated classes
    state_diff.deprecated_declared_classes = deprecated_classes_set.into_iter().collect();

    // Create the merged StateUpdate
    let merged_update = StateUpdate { block_hash, new_root, old_root, state_diff };

    Ok(merged_update)
}

async fn process_single_contract(
    contract_addr: Felt,
    storage_map: HashMap<Felt, Felt>,
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    pre_range_block: u64,
) -> Result<Option<ContractStorageDiffItem>, JobError> {
    let mut storage_entries = Vec::new();

    // Check if contract existed at pre-range block
    let contract_existed = check_contract_existed_at_block(&provider, contract_addr, pre_range_block).await;

    if contract_existed {
        // Process storage entries for existing contract
        let storage_futures: Vec<_> = storage_map
            .clone()
            .into_iter()
            .map(|(key, value)| {
                let provider = provider.clone();
                async move {
                    let pre_range_value =
                        check_pre_range_storage_value(&provider, contract_addr, key, pre_range_block).await?;

                    Ok::<_, JobError>((key, value, pre_range_value))
                }
            })
            .collect();

        // Execute all storage checks concurrently
        let results = try_join_all(storage_futures).await?;

        // Process results
        for (key, value, pre_range_value) in results {
            if pre_range_value != value {
                storage_entries.push(StorageEntry { key, value });
            }
        }
    } else {
        // Contract didn't exist, filter non-zero values
        for (key, value) in storage_map {
            if value != Felt::ZERO {
                storage_entries.push(StorageEntry { key, value });
            }
        }
    }

    if !storage_entries.is_empty() {
        Ok(Some(ContractStorageDiffItem { address: contract_addr, storage_entries }))
    } else {
        Ok(None)
    }
}

pub async fn check_contract_existed_at_block(
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    contract_address: Felt,
    block_number: u64,
) -> bool {
    match provider.get_class_hash_at(BlockId::Number(block_number), contract_address).await {
        Ok(_) => true,
        Err(_) => false,
    }
}

pub async fn check_pre_range_storage_value(
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    contract_address: Felt,
    key: Felt,
    pre_range_block: u64,
) -> Result<Felt, JobError> {
    // Get storage value at the block before our range
    match provider.get_storage_at(contract_address, key, BlockId::Number(pre_range_block)).await {
        Ok(value) => Ok(value),
        Err(e) => {
            println!(
                "Warning: Failed to get pre-range storage value for contract: {}, key: {} at block {}: {}",
                contract_address, key, pre_range_block, e
            );
            Ok(Felt::ZERO)
        }
    }
}
