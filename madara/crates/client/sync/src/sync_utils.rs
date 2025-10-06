use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Context;
use futures::{stream, StreamExt, TryStreamExt};
use mp_convert::Felt;
use mp_state_update::{ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry};
use mc_db::{MadaraBackend, MadaraStorageRead};

// Define the timing macro at the module level
macro_rules! log_elapsed {
    ($start:expr, $label:expr) => {{
        let elapsed = $start.elapsed();
        tracing::info!("Snap_Sync: {} - Elapsed time: {:?}", $label, elapsed);
    }};
}

/// squash_state_updates merge all the StateUpdate into a single StateUpdate
/// This function now performs RAW accumulation without pre_range_block checks
pub async fn squash(
    state_diffs: Vec<&StateDiff>,
    pre_range_block: Option<u64>,
    backend: Arc<MadaraBackend>,
) -> anyhow::Result<StateDiff> {

    let state_diff_map = StateDiffMap::from_state_diffs(state_diffs);

    // Convert to StateDiff without pre_range filtering
    let mut state_diff = state_diff_map.to_raw_state_diff();

    state_diff.sort();

    Ok(state_diff)
}

/// New function: Compress a raw accumulated state diff by checking against pre_range_block
/// This should be called ONCE after all accumulation is complete
pub async fn compress_state_diff(
    raw_state_diff: StateDiff,
    pre_range_block: u64,
    backend: Arc<MadaraBackend>,
) -> anyhow::Result<StateDiff> {
    let start_time = std::time::Instant::now();

    tracing::info!("Snap_Sync: Starting compression with pre_range_block={}", pre_range_block);

    // Process storage diffs with pre_range checks
    let storage_diffs = stream::iter(raw_state_diff.storage_diffs.into_iter().enumerate())
        .map(|(idx, contract_diff)| {
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

    log_elapsed!(start_time, "Storage diffs compressed");

    // Process deployed contracts and replaced classes
    let mut deployed_contracts_map: HashMap<Felt, Felt> = raw_state_diff
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
        Some(pre_range_block),
        deployed_contracts_map,
        replaced_classes_map,
    ).await?;

    log_elapsed!(start_time, "Deployed contracts and replaced classes compressed");

    let mut compressed_diff = StateDiff {
        storage_diffs,
        deployed_contracts,
        declared_classes: raw_state_diff.declared_classes,
        old_declared_contracts: raw_state_diff.old_declared_contracts,
        nonces: raw_state_diff.nonces,
        replaced_classes,
    };

    compressed_diff.sort();

    log_elapsed!(start_time, "Compression completed");

    Ok(compressed_diff)
}


const MAX_CONCURRENT_CONTRACTS_PROCESSING: usize = 400;
const MAX_CONCURRENT_GET_STORAGE_AT_CALLS: usize = 10000;
const MAX_GET_STORAGE_AT_CALL_RETRY: u64 = 3;
const MAX_GET_CLASS_HASH_AT_CALL_RETRY: u64 = 3;

#[derive(Default)]
struct StateDiffMap {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    deployed_contracts: HashMap<Felt, Felt>,
    declared_classes: HashMap<Felt, Felt>,
    deprecated_declared_classes: HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
    replaced_classes: HashMap<Felt, Felt>,
    touched_contracts: HashSet<Felt>,
}

impl StateDiffMap {
    fn from_state_diffs(ordered_state_diffs: Vec<&StateDiff>) -> Self {
        let start_time = std::time::Instant::now();
        tracing::info!("Snap_Sync: Starting from_state_diffs with {} state diffs", ordered_state_diffs.len());

        let mut state_diff_map = StateDiffMap::default();

        for (idx, state_diff) in ordered_state_diffs.iter().enumerate() {
            // Process storage diffs
            for contract_diff in &state_diff.storage_diffs {
                let contract_addr = contract_diff.address;
                state_diff_map.touched_contracts.insert(contract_addr);

                let contract_storage = state_diff_map.storage_diffs.entry(contract_addr).or_default();

                for entry in &contract_diff.storage_entries {
                    contract_storage.insert(entry.key, entry.value);
                }
            }
            log_elapsed!(start_time, format!("Iteration {}: Storage diffs processed", idx));

            // Process deployed contracts
            for item in &state_diff.deployed_contracts {
                state_diff_map.deployed_contracts.insert(item.address, item.class_hash);
            }
            log_elapsed!(start_time, format!("Iteration {}: Deployed contracts processed", idx));

            // Process declared classes
            for item in &state_diff.declared_classes {
                state_diff_map.declared_classes.insert(item.class_hash, item.compiled_class_hash);
            }
            log_elapsed!(start_time, format!("Iteration {}: Declared classes processed", idx));

            // Process nonces
            for item in &state_diff.nonces {
                state_diff_map.nonces.insert(item.contract_address, item.nonce);
            }
            log_elapsed!(start_time, format!("Iteration {}: Nonces processed", idx));

            // Process replaced classes
            for item in &state_diff.replaced_classes {
                state_diff_map.replaced_classes.insert(item.contract_address, item.class_hash);
            }
            log_elapsed!(start_time, format!("Iteration {}: Replaced classes processed", idx));

            // Process deprecated classes
            for class_hash in &state_diff.old_declared_contracts {
                state_diff_map.deprecated_declared_classes.insert(*class_hash);
            }
            log_elapsed!(start_time, format!("Iteration {}: Deprecated classes processed", idx));
        }

        tracing::info!("Snap_Sync: from_state_diffs completed - {} unique contracts touched", state_diff_map.touched_contracts.len());
        log_elapsed!(start_time, "from_state_diffs completed");

        state_diff_map
    }

    /// NEW: Convert to raw StateDiff without any pre_range filtering
    /// This is MUCH faster as it does no DB lookups
    fn to_raw_state_diff(self) -> StateDiff {
        let start_time = std::time::Instant::now();

        tracing::info!("Snap_Sync: Converting to raw state diff with {} touched contracts", self.touched_contracts.len());

        // Convert storage diffs - only include touched contracts
        let storage_diffs: Vec<ContractStorageDiffItem> = self.touched_contracts
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

        log_elapsed!(start_time, "Raw storage diffs created");

        let deployed_contracts = self
            .deployed_contracts
            .into_iter()
            .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
            .collect();

        let declared_classes = self
            .declared_classes
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();

        let nonces = self
            .nonces
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce })
            .collect();

        let replaced_classes = self
            .replaced_classes
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect();

        let deprecated_declared_classes = self.deprecated_declared_classes.into_iter().collect();

        log_elapsed!(start_time, "Raw state diff conversion completed");

        StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            old_declared_contracts: deprecated_declared_classes,
            nonces,
            replaced_classes,
        }
    }

    // REMOVED: Old get_state_diff method that did pre_range filtering
}

/// NEW: Compress a single contract's storage entries by checking against pre_range_block
async fn compress_single_contract(
    contract_addr: Felt,
    storage_entries: Vec<StorageEntry>,
    backend: Arc<MadaraBackend>,
    pre_range_block: u64,
) -> anyhow::Result<Option<ContractStorageDiffItem>> {
    let start_time = std::time::Instant::now();
    let entry_count = storage_entries.len();

    // Check if contract existed at pre_range_block
    let contract_existed = check_contract_existed_at_block(backend.clone(), contract_addr, pre_range_block).await?;

    log_elapsed!(start_time, format!("Contract {:?}: Checked existence (existed: {})", contract_addr, contract_existed));

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

    log_elapsed!(
        start_time,
        format!("Contract {:?}: Compressed {} -> {} entries", contract_addr, entry_count, compressed_entries.len())
    );

    if !compressed_entries.is_empty() {
        Ok(Some(ContractStorageDiffItem {
            address: contract_addr,
            storage_entries: compressed_entries,
        }))
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
/// It retries up to [MAX_GET_CLASS_HASH_AT_CALL_RETRY] times
/// This function does not return error when the contract does not exist
pub async fn get_class_hash_at(
    backend: Arc<MadaraBackend>,
    block_number: u64,
    contract_address: Felt,
) -> anyhow::Result<Option<Felt>> {
    let class_hash = retry_sync(
        || backend.db.get_contract_class_hash_at(block_number, &contract_address),
        MAX_GET_CLASS_HASH_AT_CALL_RETRY,
        Some(Duration::from_secs(5)),
    )
        .with_context(|| {
            format!(
                "Failed to get class hash for contract: {} at block {}",
                contract_address, block_number
            )
        })?;

    Ok(class_hash)
}


pub fn retry_sync<F, T, E>(mut func: F, max_retries: u64, delay: Option<Duration>) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
{
    let mut attempts = 0;
    loop {
        match func() {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempts += 1;
                if attempts >= max_retries {
                    return Err(e);
                }
                if let Some(d) = delay {
                    std::thread::sleep(d);
                }
            }
        }
    }
}


pub async fn check_pre_range_storage_value(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    key: Felt,
    pre_range_block: u64,
) -> Option<Felt> {
    tokio::task::spawn_blocking(move || {
        retry_sync(
            || backend.db.get_storage_at(pre_range_block, &contract_address, &key),
            MAX_GET_STORAGE_AT_CALL_RETRY,
            Some(Duration::from_secs(5)),
        )
        .ok()
        .flatten()
    })
    .await
    .ok()
    .flatten()
}
