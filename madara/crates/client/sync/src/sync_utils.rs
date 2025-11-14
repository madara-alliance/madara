use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use anyhow::Context;
use futures::{stream, StreamExt, TryStreamExt};
use mp_convert::Felt;
use mp_state_update::{ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry};
use mc_db::{MadaraBackend, MadaraStorageRead};

const MAX_CONCURRENT_CONTRACTS_PROCESSING: usize = 400;
const MAX_CONCURRENT_GET_STORAGE_AT_CALLS: usize = 10000;

/// Compress a raw accumulated state diff by checking against pre_range_block
/// This should be called ONCE after all accumulation is complete
pub async fn compress_state_diff(
    raw_state_diff: StateDiff,
    pre_range_block: Option<u64>,
    backend: Arc<MadaraBackend>,
) -> anyhow::Result<StateDiff> {

    // Process storage diffs with pre_range checks
    let storage_diffs = stream::iter(raw_state_diff.storage_diffs.into_iter().enumerate())
        .map(|(_idx, contract_diff)| {
            let backend = backend.clone();
            async move {
                compress_single_contract(
                    contract_diff.address,
                    contract_diff.storage_entries,
                    backend,
                    pre_range_block
                ).await
            }
        })
        .buffer_unordered(MAX_CONCURRENT_CONTRACTS_PROCESSING)
        .try_filter_map(|contract_storage_diff| async move { Ok(contract_storage_diff) })
        .try_collect::<Vec<_>>()
        .await?;

    // Process deployed contracts and replaced classes
    let deployed_contracts_map: HashMap<Felt, Felt> = raw_state_diff
        .deployed_contracts
        .into_iter()
        .map(|item| (item.address, item.class_hash))
        .collect();

    let replaced_classes_map: HashMap<Felt, Felt> = raw_state_diff
        .replaced_classes
        .into_iter()
        .map(|item| (item.contract_address, item.class_hash))
        .collect();

    let (replaced_classes, deployed_contracts) = process_deployed_contracts_and_replaced_classes(
        backend.clone(),
        pre_range_block,
        deployed_contracts_map,
        replaced_classes_map,
    ).await?;

    let mut compressed_diff = StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes: raw_state_diff.declared_classes,
        old_declared_contracts: raw_state_diff.old_declared_contracts,
        nonces: raw_state_diff.nonces,
        replaced_classes,
    };

    compressed_diff.sort();

    Ok(compressed_diff)
}



#[derive(Default)]
pub struct StateDiffMap {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    deployed_contracts: HashMap<Felt, Felt>,
    declared_classes: HashMap<Felt, Felt>,
    deprecated_declared_classes: HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
    replaced_classes: HashMap<Felt, Felt>,
    touched_contracts: HashSet<Felt>,
}

impl StateDiffMap {

    /// Apply a single state diff to the existing StateDiffMap
    pub fn apply_state_diff(&mut self, state_diff: &StateDiff) {

        // Process storage diffs
        for contract_diff in &state_diff.storage_diffs {
            let contract_addr = contract_diff.address;
            self.touched_contracts.insert(contract_addr);

            let contract_storage = self.storage_diffs.entry(contract_addr).or_default();

            for entry in &contract_diff.storage_entries {
                contract_storage.insert(entry.key, entry.value);
            }
        }
        // Process deployed contracts
        for item in &state_diff.deployed_contracts {
            self.deployed_contracts.insert(item.address, item.class_hash);
        }

        // Process declared classes
        for item in &state_diff.declared_classes {
            self.declared_classes.insert(item.class_hash, item.compiled_class_hash);
        }

        // Process nonces
        for item in &state_diff.nonces {
            self.nonces.insert(item.contract_address, item.nonce);
        }

        // Process replaced classes
        for item in &state_diff.replaced_classes {
            self.replaced_classes.insert(item.contract_address, item.class_hash);
        }

        // Process deprecated classes
        for class_hash in &state_diff.old_declared_contracts {
            self.deprecated_declared_classes.insert(*class_hash);
        }
    }

    /// Convert to raw StateDiff without any pre_range filtering
    /// This is MUCH faster as it does no DB lookups
    pub fn to_raw_state_diff(&self) -> StateDiff {

        // Convert storage diffs - only include touched contracts
        let storage_diffs: Vec<ContractStorageDiffItem> = self.touched_contracts.clone()
            .into_iter()
            .filter_map(|contract_addr| {
                self.storage_diffs.get(&contract_addr).map(|storage_map| {
                    let storage_entries: Vec<StorageEntry> = storage_map
                        .iter()
                        .map(|(key, value)| StorageEntry { key: *key, value: *value })
                        .collect();

                    ContractStorageDiffItem {
                        address: contract_addr,
                        storage_entries,
                    }
                })
            })
            .filter(|item| !item.storage_entries.is_empty())
            .collect();


        let deployed_contracts = self
            .deployed_contracts.clone()
            .into_iter()
            .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
            .collect();

        let declared_classes = self
            .declared_classes.clone()
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();

        let nonces = self
            .nonces.clone()
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce })
            .collect();

        let replaced_classes = self
            .replaced_classes.clone()
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect();

        let deprecated_declared_classes = self.deprecated_declared_classes.clone().into_iter().collect();

        StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            old_declared_contracts: deprecated_declared_classes,
            nonces,
            replaced_classes,
        }
    }
}

/// Compress a single contract's storage entries by checking against pre_range_block
async fn compress_single_contract(
    contract_addr: Felt,
    storage_entries: Vec<StorageEntry>,
    backend: Arc<MadaraBackend>,
    pre_range_block_option: Option<u64>,
) -> anyhow::Result<Option<ContractStorageDiffItem>> {
    let storage_entries = match pre_range_block_option {
        Some(pre_range_block) => {
            // Check if contract existed at pre_range_block
            let contract_existed = check_contract_existed_at_block(backend.clone(), contract_addr, pre_range_block).await?;

            let compressed_entries = if contract_existed {
                // Contract existed - check each storage entry against pre_range value
                stream::iter(storage_entries)
                    .map(|entry| {
                        let backend = backend.clone();
                        async move {
                            let pre_range_value = check_pre_range_storage_value(
                                backend,
                                contract_addr,
                                entry.key,
                                pre_range_block
                            ).await;
                            (entry, pre_range_value)
                        }
                    })
                    .buffer_unordered(MAX_CONCURRENT_GET_STORAGE_AT_CALLS)
                    .filter_map(|(entry, pre_range_value)| async move {
                        // Only keep if value changed
                        if pre_range_value != Some(entry.value) {
                            Some(entry)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .await
            } else {
                // Contract didn't exist - filter out zero values (default state)
                storage_entries
                    .into_iter()
                    .filter(|entry| entry.value != Felt::ZERO)
                    .collect()
            };

            compressed_entries
        }
        None => {
            // No pre_range_block - filter out zero values (default state)
            storage_entries
                .into_iter()
                .filter(|entry| entry.value != Felt::ZERO)
                .collect()
        }
    };

    if !storage_entries.is_empty() {
        Ok(Some(ContractStorageDiffItem { address: contract_addr, storage_entries }))
    } else {
        Ok(None)
    }
}

/// Process a class hash and does the following:
/// 1. Remove all the contracts from replaced classes which are also deployed and update the final class hash in the deployed contracts map
/// 2. Check the previous class hash for all remaining contracts in replaced_class_hashes
/// 3. If they are the same, remove them from the mapping
async fn process_deployed_contracts_and_replaced_classes(
    backend: Arc<MadaraBackend>,
    pre_range_block_option: Option<u64>,
    mut deployed_contracts: HashMap<Felt, Felt>,
    mut replaced_class_hashes: HashMap<Felt, Felt>,
) -> anyhow::Result<(Vec<ReplacedClassItem>, Vec<DeployedContractItem>)> {
    // Loop through all replaced_class_hashes and check if they exist in the deployed_contracts
    // Remove the contracts from replaced_class_hashes if they exist in deployed_contracts
    replaced_class_hashes.retain(|contract_address, class_hash| {
        match deployed_contracts.get_mut(contract_address) {
            Some(existing_class_hash) => {
                // replace the class hash in deployed_contracts
                *existing_class_hash = *class_hash;
                // remove
                false
            }
            // keep
            None => true,
        }
    });

    let replaced_class_hash_items: Vec<ReplacedClassItem> = match pre_range_block_option {
        Some(pre_range_block) => {
            stream::iter(replaced_class_hashes)
                .map(|(contract_address, class_hash)| {
                    let backend = backend.clone();
                    async move {
                        process_class(backend, pre_range_block, contract_address, class_hash).await
                    }
                })
                .buffer_unordered(MAX_CONCURRENT_CONTRACTS_PROCESSING)
                .try_filter_map(|(contract_address, class_hash)| async move {
                    Ok(class_hash.map(|class_hash| ReplacedClassItem { contract_address, class_hash }))
                })
                .try_collect::<Vec<_>>()
                .await?
        }
        None => {
            // No pre_range_block - include all replaced classes
            replaced_class_hashes
                .into_iter()
                .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
                .collect()
        }
    };

    let deployed_contract_items: Vec<DeployedContractItem> = deployed_contracts
        .into_iter()
        .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
        .collect();

    Ok((replaced_class_hash_items, deployed_contract_items))
}

async fn process_class(
    backend: Arc<MadaraBackend>,
    pre_range_block: u64,
    contract_address: Felt,
    class_hash: Felt,
) -> anyhow::Result<(Felt, Option<Felt>)> {
    match get_class_hash_at(backend.clone(), pre_range_block, contract_address).await? {
        Some(prev_class_hash) => {
            if prev_class_hash == class_hash {
                Ok((contract_address, None))
            } else {
                Ok((contract_address, Some(class_hash)))
            }
        }
        None => Ok((contract_address, Some(class_hash))),
    }
}

/// This function tells if the contract existed at the given block number
pub async fn check_contract_existed_at_block(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    block_number: u64,
) -> anyhow::Result<bool> {
    let x = get_class_hash_at(backend.clone(), block_number, contract_address).await?.is_some();
    Ok(x)
}

/// This function returns the class hash of a contract at a given block number
/// If it exists, it returns the class hash along with `exists=true`
/// If it doesn't exist, it returns the zero-class hash along with `exists=false`
/// This function does not return error when the contract does not exist
pub async fn get_class_hash_at(
    backend: Arc<MadaraBackend>,
    block_number: u64,
    contract_address: Felt,
) -> anyhow::Result<Option<Felt>> {
    backend.db.get_contract_class_hash_at(block_number, &contract_address)
        .with_context(|| {
            format!(
                "Failed to get class hash for contract: {} at block {}",
                contract_address, block_number
            )
        })
}

pub async fn check_pre_range_storage_value(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    key: Felt,
    pre_range_block: u64,
) -> Option<Felt> {
    tokio::task::spawn_blocking(move || {
        backend.db.get_storage_at(pre_range_block, &contract_address, &key).ok().flatten()
    })
        .await
        .ok()
        .flatten()
}
