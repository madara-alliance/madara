use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Context;
use futures::{stream, StreamExt, TryStreamExt};
use mp_convert::Felt;
use mp_state_update::{ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry};
use mc_db::{MadaraBackend, MadaraStorageRead};


/// squash_state_updates merge all the StateUpdate into a single StateUpdate
/// TODO: might be able to change this to take all apply once if ram allows and speed is better !
pub async fn squash(
    state_diffs: Vec<&StateDiff>,
    pre_range_block: Option<u64>,
    backend: Arc<MadaraBackend>,
) -> anyhow::Result<StateDiff> {

    let state_diff_map = StateDiffMap::from_state_diffs(state_diffs);
    let mut state_diff = state_diff_map.get_state_diff(pre_range_block, backend.clone()).await?;

    state_diff.sort();

    Ok(state_diff)
}




const MAX_CONCURRENT_CONTRACTS_PROCESSING: usize = 40;
const MAX_CONCURRENT_GET_STORAGE_AT_CALLS: usize = 100;
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
}

impl StateDiffMap {
    fn from_state_diffs(ordered_state_diffs: Vec<&StateDiff>) -> Self {
        // Maps to efficiently track the latest state
        let mut state_diff_map = StateDiffMap::default();

        // Process each update in order
        for state_diff in ordered_state_diffs {
            // Process storage diffs
            for contract_diff in &state_diff.storage_diffs {
                let contract_addr = contract_diff.address;
                let contract_storage = state_diff_map.storage_diffs.entry(contract_addr).or_default();

                for entry in &contract_diff.storage_entries {
                    contract_storage.insert(entry.key, entry.value);
                }
            }

            // Process deployed contracts
            for item in &state_diff.deployed_contracts {
                state_diff_map.deployed_contracts.insert(item.address, item.class_hash);
            }

            // Process declared classes
            for item in &state_diff.declared_classes {
                state_diff_map.declared_classes.insert(item.class_hash, item.compiled_class_hash);
            }

            // Process nonces
            for item in &state_diff.nonces {
                state_diff_map.nonces.insert(item.contract_address, item.nonce);
            }

            // Process replaced classes
            for item in &state_diff.replaced_classes {
                state_diff_map.replaced_classes.insert(item.contract_address, item.class_hash);
            }

            // Process deprecated classes
            for class_hash in &state_diff.old_declared_contracts {
                state_diff_map.deprecated_declared_classes.insert(*class_hash);
            }
        }

        state_diff_map
    }

    async fn get_state_diff(
        self,
        pre_range_block: Option<u64>,
        backend: Arc<MadaraBackend>,
    ) -> anyhow::Result<StateDiff> {
        // Processing all contracts in parallel.
        // The idea is that it might be the case that for a contract, a particular storage slot is
        // changed twice to finally have the original value, in which case the new final value is not
        // different from the value in the previous batch and hence it shouldn't be in the storage diff
        // The result is the storage diff of all the contracts

        let storage_diffs = stream::iter(self.storage_diffs)
            .map(|(contract_addr, storage_map)| {
                let backend = backend.clone();
                async move {
                    process_single_contract(contract_addr, storage_map, backend, pre_range_block).await
                }
            })
            .buffer_unordered(MAX_CONCURRENT_CONTRACTS_PROCESSING)
            .try_filter_map(|contract_storage_diff| async move { Ok(contract_storage_diff) })
            .try_collect::<Vec<_>>()
            .await?;

        // Processing deployed contracts and replaced classes
        // The idea is that it might be the case that a class is replaced twice to the original value,
        // in which case we shouldn't put it in replaced classes
        // Secondly, it might also be possible that a contract is deployed and its class is replaced
        // in the same batch, in which case we should just update the class hash in deployed contracts
        // and remove it from the replaced class map
        let (replaced_classes, deployed_contracts) = process_deployed_contracts_and_replaced_classes(
            backend.clone(),
            pre_range_block,
            self.deployed_contracts,
            self.replaced_classes,
        )
            .await?;

        // Declared classes
        let declared_classes = self
            .declared_classes
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();

        // Nonces
        let nonces =
            self.nonces.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect();

        // Deprecated classes
        let deprecated_declared_classes = self.deprecated_declared_classes.into_iter().collect();

        Ok(StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            old_declared_contracts: deprecated_declared_classes,
            nonces,
            replaced_classes,
        })
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
        None => Vec::new(),
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

/// Processes the storage of a single contract to do the following
/// 1. Check if the contract existed in the `pre_range_block`
/// 2. If yes, check the value of all keys in the storage map of this contract in the `pre_range_block`
/// 3. If no, filter the non-zero values in the storage map
async fn process_single_contract(
    contract_addr: Felt,
    storage_map: HashMap<Felt, Felt>,
    backend: Arc<MadaraBackend>,
    pre_range_block_option: Option<u64>,
) -> anyhow::Result<Option<ContractStorageDiffItem>> {
    let storage_entries = match pre_range_block_option {
        None => {
            // pre_range_block is not available, filter non-zero values
            // We don't need zero values if this is the first block (i.e., pre_range_block doesn't exist)
            // since zero is the default value
            storage_map
                .into_iter()
                .filter(|(_, value)| *value != Felt::ZERO)
                .map(|(key, value)| StorageEntry { key, value })
                .collect()
        }
        Some(pre_range_block) => {
            if check_contract_existed_at_block(backend.clone(), contract_addr, pre_range_block).await? {
                // Process storage entries only for an existing contract
                stream::iter(storage_map)
                    .map(|(key, value)| {
                        let backend = backend.clone();
                        async move {
                            let pre_range_value =
                                check_pre_range_storage_value(backend, contract_addr, key, pre_range_block);
                            (key, value, pre_range_value)
                        }
                    })
                    .buffer_unordered(MAX_CONCURRENT_GET_STORAGE_AT_CALLS)
                    .filter_map(|(key, value, pre_range_value)| async move {
                        if pre_range_value != Some(value) {
                            Some(StorageEntry { key, value })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .await
            } else {
                // Contract didn't exist, filter non-zero values
                // We don't need zero values if the contract didn't exist at the pre-range block
                // since zero is the default value
                storage_map
                    .into_iter()
                    .filter(|(_, value)| *value != Felt::ZERO)
                    .map(|(key, value)| StorageEntry { key, value })
                    .collect()
            }
        }
    };

    if !storage_entries.is_empty() {
        Ok(Some(ContractStorageDiffItem { address: contract_addr, storage_entries }))
    } else {
        Ok(None)
    }
}

/// This function tells if the contract existed at the given block number
pub async fn check_contract_existed_at_block(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    block_number: u64,
) -> anyhow::Result<bool> {
    let x = get_class_hash_at(backend.clone(), block_number, contract_address).await?.is_some();
    // println!("Contract : {:?} {} for block number : {:?}",  contract_address, x, block_number);
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

    // println!("TESTING SYNC-SNAP : {:?} has class hash : {:?}", contract_address, class_hash);
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

/// This function returns the storage value of a key at a given block number
/// It retries up to [MAX_GET_STORAGE_AT_CALL_RETRY] times
pub fn check_pre_range_storage_value(
    backend: Arc<MadaraBackend>,
    contract_address: Felt,
    key: Felt,
    pre_range_block: u64,
) -> Option<Felt> {
    retry_sync(
        || backend.db.get_storage_at(pre_range_block, &contract_address, &key),
        MAX_GET_STORAGE_AT_CALL_RETRY,
        Some(Duration::from_secs(5)),
    )
        .ok()
        .flatten()
}