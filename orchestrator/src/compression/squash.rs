use crate::compression::batch_rpc::{BatchRpcClient, BatchRpcError};
use crate::compression::utils::sort_state_diff;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use color_eyre::eyre::eyre;
use starknet_core::types::{
    BlockId, ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, Felt,
    NonceUpdate, ReplacedClassItem, StateDiff, StateUpdate, StorageEntry,
};
use std::collections::{HashMap, HashSet};
use tracing::debug;

/// squash_state_updates merge all the StateUpdate into a single StateUpdate
///
/// This function uses batch JSON-RPC calls to efficiently fetch pre-range block
/// storage values and class hashes, reducing the number of HTTP requests by ~10x.
pub async fn squash(
    state_updates: Vec<&StateUpdate>,
    pre_range_block: Option<u64>,
    batch_client: &BatchRpcClient,
) -> Result<StateUpdate, JobError> {
    debug!("Squashing state updates");
    if state_updates.is_empty() {
        return Err(JobError::Other(OtherError(eyre!("Cannot merge empty state updates"))));
    }

    // Take the last block hash and number from the last update as our "latest"
    let last_state_update = state_updates.last().ok_or(JobError::Other(OtherError(eyre!("Invalid state updates"))))?;
    let block_hash = last_state_update.block_hash;
    let new_root = last_state_update.new_root;
    let old_root = state_updates.first().ok_or(JobError::Other(OtherError(eyre!("Invalid state updates"))))?.old_root;

    // Collecting a simplified squashed state diff map
    let state_diff_map = StateDiffMap::from_state_update(state_updates);

    let state_diff = state_diff_map.get_state_diff_batched(pre_range_block, batch_client).await?;

    // Create the merged StateUpdate
    let mut merged_update = StateUpdate { block_hash, new_root, old_root, state_diff };

    // Sort the merged StateUpdate
    sort_state_diff(&mut merged_update);

    debug!("Squashed state updates");

    Ok(merged_update)
}

#[derive(Default)]
struct StateDiffMap {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    deployed_contracts: HashMap<Felt, Felt>,
    declared_classes: HashMap<Felt, Felt>,
    deprecated_declared_classes: HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
    replaced_classes: HashMap<Felt, Felt>,
    // Migrated classes (SNIP-34): class_hash -> new BLAKE compiled_class_hash
    // migrated_compiled_classes: HashMap<Felt, Felt>,
}

impl StateDiffMap {
    fn from_state_update(state_updates: Vec<&StateUpdate>) -> Self {
        // Maps to efficiently track the latest state
        let mut state_diff_map = StateDiffMap::default();

        // Process each update in order
        for update in state_updates {
            // Process storage diffs
            for contract_diff in &update.state_diff.storage_diffs {
                let contract_addr = contract_diff.address;
                let contract_storage = state_diff_map.storage_diffs.entry(contract_addr).or_default();

                for entry in &contract_diff.storage_entries {
                    contract_storage.insert(entry.key, entry.value);
                }
            }

            // Process deployed contracts
            for item in &update.state_diff.deployed_contracts {
                state_diff_map.deployed_contracts.insert(item.address, item.class_hash);
            }

            // Process declared classes
            for item in &update.state_diff.declared_classes {
                state_diff_map.declared_classes.insert(item.class_hash, item.compiled_class_hash);
            }

            // Process nonces
            for item in &update.state_diff.nonces {
                state_diff_map.nonces.insert(item.contract_address, item.nonce);
            }

            // Process replaced classes
            for item in &update.state_diff.replaced_classes {
                state_diff_map.replaced_classes.insert(item.contract_address, item.class_hash);
            }

            // Process deprecated classes
            for class_hash in &update.state_diff.deprecated_declared_classes {
                state_diff_map.deprecated_declared_classes.insert(*class_hash);
            }

            // Process migrated classes (SNIP-34)
            // if let Some(ref migrated) = update.state_diff.migrated_compiled_classes {
            //     for item in migrated {
            //         state_diff_map.migrated_compiled_classes.insert(item.class_hash, item.compiled_class_hash);
            //     }
            // }
        }

        state_diff_map
    }

    /// Get the state diff using batched RPC calls for efficiency.
    ///
    /// This method batches all RPC calls to reduce HTTP overhead:
    /// 1. First, batch-fetch class hashes to determine which contracts existed
    /// 2. Then, batch-fetch storage values for existing contracts
    /// 3. Finally, batch-fetch class hashes for replaced classes
    async fn get_state_diff_batched(
        self,
        pre_range_block: Option<u64>,
        batch_client: &BatchRpcClient,
    ) -> Result<StateDiff, JobError> {
        // Process storage diffs using batched calls
        let storage_diffs = process_storage_diffs_batched(&self.storage_diffs, pre_range_block, batch_client).await?;

        // Process deployed contracts and replaced classes using batched calls
        let (replaced_classes, deployed_contracts) = process_deployed_contracts_and_replaced_classes_batched(
            batch_client,
            pre_range_block,
            self.deployed_contracts,
            self.replaced_classes,
        )
        .await?;

        // Declared classes - no RPC calls needed
        let declared_classes = self
            .declared_classes
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();

        // Nonces - no RPC calls needed
        let nonces =
            self.nonces.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect();

        // Deprecated classes - no RPC calls needed
        let deprecated_declared_classes = self.deprecated_declared_classes.into_iter().collect();

        // // Convert migrated classes back to Vec
        // let migrated_compiled_classes: Vec<MigratedCompiledClassItem> = self
        //     .migrated_compiled_classes
        //     .into_iter()
        //     .map(|(class_hash, compiled_class_hash)| MigratedCompiledClassItem { class_hash, compiled_class_hash })
        //     .collect();

        Ok(StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            deprecated_declared_classes,
            nonces,
            replaced_classes,
            // migrated_compiled_classes: if migrated_compiled_classes.is_empty() {
            //     None
            // } else {
            //     Some(migrated_compiled_classes)
            // },
        })
    }
}

/// Process storage diffs using batched RPC calls.
///
/// For each contract with storage changes:
/// 1. Check if the contract existed at pre_range_block
/// 2. For existing contracts, fetch all pre-range storage values
/// 3. Filter out entries where the value hasn't changed
/// 4. For new contracts (didn't exist), filter out zero values
async fn process_storage_diffs_batched(
    storage_diffs: &HashMap<Felt, HashMap<Felt, Felt>>,
    pre_range_block: Option<u64>,
    batch_client: &BatchRpcClient,
) -> Result<Vec<ContractStorageDiffItem>, JobError> {
    let pre_range_block = match pre_range_block {
        Some(block) => block,
        None => {
            // No pre-range block, just filter out zero values for all contracts
            return Ok(storage_diffs
                .iter()
                .filter_map(|(contract_addr, storage_map)| {
                    let entries: Vec<StorageEntry> = storage_map
                        .iter()
                        .filter(|(_, value)| **value != Felt::ZERO)
                        .map(|(key, value)| StorageEntry { key: *key, value: *value })
                        .collect();

                    (!entries.is_empty())
                        .then_some(ContractStorageDiffItem { address: *contract_addr, storage_entries: entries })
                })
                .collect());
        }
    };

    let block_id = BlockId::Number(pre_range_block);

    // Step 1: Batch-fetch class hashes to determine which contracts existed
    let all_contracts: Vec<Felt> = storage_diffs.keys().copied().collect();
    debug!("Checking existence of {} contracts at block {}", all_contracts.len(), pre_range_block);

    let contract_existence = batch_client
        .batch_get_class_hash_at(all_contracts.clone(), block_id)
        .await
        .map_err(|e| JobError::ProviderError(format!("Failed to batch get class hashes: {}", e)))?;

    // Separate contracts into existing and new
    let mut existing_contracts = Vec::new();
    let mut new_contracts = Vec::new();
    for addr in &all_contracts {
        if contract_existence.get(addr).map(|v| v.is_some()).unwrap_or(false) {
            existing_contracts.push(*addr);
        } else {
            new_contracts.push(*addr);
        }
    }

    debug!("Found {} existing contracts and {} new contracts", existing_contracts.len(), new_contracts.len());

    // Step 2: For existing contracts, collect all storage queries
    let storage_queries: Vec<(Felt, Felt)> = existing_contracts
        .iter()
        .flat_map(|contract_addr| {
            storage_diffs
                .get(contract_addr)
                .map(|storage_map| storage_map.keys().map(|key| (*contract_addr, *key)).collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .collect();

    debug!("Fetching {} pre-range storage values", storage_queries.len());

    // Step 3: Batch-fetch all pre-range storage values
    let pre_range_values = if storage_queries.is_empty() {
        HashMap::new()
    } else {
        batch_client
            .batch_get_storage_at(storage_queries, block_id)
            .await
            .map_err(|e| JobError::ProviderError(format!("Failed to batch get storage values: {}", e)))?
    };

    // Step 4: Build storage diffs by comparing values
    // For both existing and new contracts, we compare against pre_range_values.
    // For new contracts, pre_range_values won't have entries, defaulting to ZERO.
    let result: Vec<ContractStorageDiffItem> = all_contracts
        .iter()
        .filter_map(|contract_addr| {
            storage_diffs
                .get(contract_addr)
                .and_then(|storage_map| build_storage_diff_item(*contract_addr, storage_map, &pre_range_values))
        })
        .collect();

    Ok(result)
}

/// Build a ContractStorageDiffItem by comparing new values against pre-range values.
/// Only includes entries where the value has changed (pre-range defaults to ZERO if not found).
/// Returns None if no entries changed.
///
/// This is a pure function that can be unit tested without mocking RPC calls.
fn build_storage_diff_item(
    contract_addr: Felt,
    storage_map: &HashMap<Felt, Felt>,
    pre_range_values: &HashMap<(Felt, Felt), Felt>,
) -> Option<ContractStorageDiffItem> {
    let entries: Vec<StorageEntry> = storage_map
        .iter()
        .filter_map(|(key, new_value)| {
            let pre_range_value = pre_range_values.get(&(contract_addr, *key)).copied().unwrap_or(Felt::ZERO);
            if pre_range_value != *new_value {
                Some(StorageEntry { key: *key, value: *new_value })
            } else {
                None
            }
        })
        .collect();

    (!entries.is_empty()).then_some(ContractStorageDiffItem { address: contract_addr, storage_entries: entries })
}

/// Filter replaced classes to only include those where the class hash actually changed.
/// Returns None for contracts that didn't exist before or where the class hash is the same.
///
/// This is a pure function that can be unit tested without mocking RPC calls.
fn filter_changed_replaced_classes(
    replaced_class_hashes: HashMap<Felt, Felt>,
    prev_class_hashes: &HashMap<Felt, Option<Felt>>,
) -> Vec<ReplacedClassItem> {
    replaced_class_hashes
        .into_iter()
        .filter_map(|(contract_address, new_class_hash)| {
            let prev_class_hash = prev_class_hashes.get(&contract_address).copied().flatten();

            match prev_class_hash {
                Some(prev) if prev == new_class_hash => None, // Class didn't change
                _ => Some(ReplacedClassItem { contract_address, class_hash: new_class_hash }),
            }
        })
        .collect()
}

/// Process deployed contracts and replaced classes using batched RPC calls.
///
/// 1. Remove contracts from replaced_classes if they're also in deployed_contracts
///    (update the class hash in deployed_contracts instead)
/// 2. Batch-fetch class hashes for remaining replaced_class contracts
/// 3. Filter out replaced classes where the class hasn't actually changed
async fn process_deployed_contracts_and_replaced_classes_batched(
    batch_client: &BatchRpcClient,
    pre_range_block: Option<u64>,
    mut deployed_contracts: HashMap<Felt, Felt>,
    mut replaced_class_hashes: HashMap<Felt, Felt>,
) -> Result<(Vec<ReplacedClassItem>, Vec<DeployedContractItem>), JobError> {
    // Remove contracts from replaced_class_hashes if they exist in deployed_contracts
    // and update the deployed contract's class hash
    replaced_class_hashes.retain(|contract_address, class_hash| match deployed_contracts.get_mut(contract_address) {
        Some(existing_class_hash) => {
            *existing_class_hash = *class_hash;
            false // Remove from replaced_class_hashes
        }
        None => true, // Keep in replaced_class_hashes
    });

    // Build replaced class items
    let replaced_class_hash_items: Vec<ReplacedClassItem> = match pre_range_block {
        Some(pre_range_block) => {
            if replaced_class_hashes.is_empty() {
                Vec::new()
            } else {
                // Batch-fetch class hashes for all contracts in replaced_class_hashes
                let contracts: Vec<Felt> = replaced_class_hashes.keys().copied().collect();
                let block_id = BlockId::Number(pre_range_block);

                debug!("Checking previous class hashes for {} replaced contracts", contracts.len());

                let prev_class_hashes = batch_client
                    .batch_get_class_hash_at(contracts, block_id)
                    .await
                    .map_err(|e| JobError::ProviderError(format!("Failed to batch get class hashes: {}", e)))?;

                // Filter: only include if class actually changed
                filter_changed_replaced_classes(replaced_class_hashes, &prev_class_hashes)
            }
        }
        None => Vec::new(),
    };

    let deployed_contract_items: Vec<DeployedContractItem> = deployed_contracts
        .into_iter()
        .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
        .collect();

    Ok((replaced_class_hash_items, deployed_contract_items))
}

impl From<BatchRpcError> for JobError {
    fn from(err: BatchRpcError) -> Self {
        JobError::ProviderError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_update_with_storage(
        block_hash: &str,
        old_root: &str,
        new_root: &str,
        storage: Vec<(&str, Vec<(&str, &str)>)>,
    ) -> StateUpdate {
        StateUpdate {
            block_hash: Felt::from_hex(block_hash).unwrap(),
            old_root: Felt::from_hex(old_root).unwrap(),
            new_root: Felt::from_hex(new_root).unwrap(),
            state_diff: StateDiff {
                storage_diffs: storage
                    .into_iter()
                    .map(|(addr, entries)| ContractStorageDiffItem {
                        address: Felt::from_hex(addr).unwrap(),
                        storage_entries: entries
                            .into_iter()
                            .map(|(k, v)| StorageEntry {
                                key: Felt::from_hex(k).unwrap(),
                                value: Felt::from_hex(v).unwrap(),
                            })
                            .collect(),
                    })
                    .collect(),
                deployed_contracts: vec![],
                declared_classes: vec![],
                deprecated_declared_classes: vec![],
                nonces: vec![],
                replaced_classes: vec![],
                migrated_compiled_classes: None,
            },
        }
    }

    #[test]
    fn test_state_diff_map_from_single_update() {
        let state_update = state_update_with_storage("0x123", "0x1", "0x2", vec![("0x100", vec![("0x1", "0x10")])]);

        let state_diff_map = StateDiffMap::from_state_update(vec![&state_update]);

        assert_eq!(state_diff_map.storage_diffs.len(), 1);
        let contract_storage = state_diff_map.storage_diffs.get(&Felt::from_hex("0x100").unwrap()).unwrap();
        assert_eq!(contract_storage.get(&Felt::from_hex("0x1").unwrap()), Some(&Felt::from_hex("0x10").unwrap()));
    }

    #[test]
    fn test_state_diff_map_merges_multiple_updates() {
        let update1 = state_update_with_storage("0x123", "0x1", "0x2", vec![("0x100", vec![("0x1", "0x10")])]);
        let update2 = state_update_with_storage("0x456", "0x2", "0x3", vec![("0x100", vec![("0x1", "0x20")])]);

        let state_diff_map = StateDiffMap::from_state_update(vec![&update1, &update2]);

        // Should have the final value from update2
        let contract_storage = state_diff_map.storage_diffs.get(&Felt::from_hex("0x100").unwrap()).unwrap();
        assert_eq!(
            contract_storage.get(&Felt::from_hex("0x1").unwrap()),
            Some(&Felt::from_hex("0x20").unwrap()) // Latest value
        );
    }

    // ========================================================================
    // Tests for pure comparison functions
    // ========================================================================

    /// Comprehensive test for build_storage_diff_item covering all edge cases:
    /// - Existing contract: unchanged values filtered, changed values included
    /// - New contract (no pre-range): zero values filtered, non-zero included
    /// - Returns None when all entries filtered out
    #[test]
    fn test_build_storage_diff_item() {
        let contract_addr = Felt::from_hex("0x100").unwrap();
        let key1 = Felt::from_hex("0x1").unwrap();
        let key2 = Felt::from_hex("0x2").unwrap();
        let key3 = Felt::from_hex("0x3").unwrap();
        let key4 = Felt::from_hex("0x4").unwrap();

        // Case 1: Existing contract with mixed changes
        {
            let mut storage_map = HashMap::new();
            storage_map.insert(key1, Felt::from_hex("0x10").unwrap()); // Same as pre-range (filtered)
            storage_map.insert(key2, Felt::from_hex("0x30").unwrap()); // Changed from 0x20 (included)
            storage_map.insert(key3, Felt::from_hex("0x40").unwrap()); // New key, no pre-range (included)
            storage_map.insert(key4, Felt::ZERO); // New key but ZERO (filtered)

            let mut pre_range_values = HashMap::new();
            pre_range_values.insert((contract_addr, key1), Felt::from_hex("0x10").unwrap());
            pre_range_values.insert((contract_addr, key2), Felt::from_hex("0x20").unwrap());

            let result = build_storage_diff_item(contract_addr, &storage_map, &pre_range_values);
            assert!(result.is_some());
            let diff = result.unwrap();
            assert_eq!(diff.address, contract_addr);

            let entry_map: HashMap<Felt, Felt> = diff.storage_entries.iter().map(|e| (e.key, e.value)).collect();
            assert_eq!(entry_map.len(), 2);
            assert!(!entry_map.contains_key(&key1)); // Unchanged - filtered
            assert_eq!(entry_map.get(&key2), Some(&Felt::from_hex("0x30").unwrap())); // Changed
            assert_eq!(entry_map.get(&key3), Some(&Felt::from_hex("0x40").unwrap())); // New key
            assert!(!entry_map.contains_key(&key4)); // ZERO - filtered
        }

        // Case 2: New contract (empty pre-range) - filters zeros only
        {
            let mut storage_map = HashMap::new();
            storage_map.insert(key1, Felt::from_hex("0x10").unwrap()); // Non-zero (included)
            storage_map.insert(key2, Felt::ZERO); // Zero (filtered)

            let pre_range_values = HashMap::new(); // Empty - simulates new contract

            let result = build_storage_diff_item(contract_addr, &storage_map, &pre_range_values);
            assert!(result.is_some());
            let diff = result.unwrap();
            assert_eq!(diff.storage_entries.len(), 1);
            assert_eq!(diff.storage_entries[0].key, key1);
        }

        // Case 3: Returns None when all entries are filtered
        {
            let mut storage_map = HashMap::new();
            storage_map.insert(key1, Felt::ZERO); // Zero with no pre-range (filtered)
            storage_map.insert(key2, Felt::from_hex("0x10").unwrap()); // Same as pre-range (filtered)

            let mut pre_range_values = HashMap::new();
            pre_range_values.insert((contract_addr, key2), Felt::from_hex("0x10").unwrap());

            let result = build_storage_diff_item(contract_addr, &storage_map, &pre_range_values);
            assert!(result.is_none());
        }
    }

    /// Comprehensive test for filter_changed_replaced_classes covering all edge cases:
    /// - Unchanged class hash (filtered)
    /// - Changed class hash (included)
    /// - New contract with None prev (included)
    /// - Contract missing from prev map (included)
    /// - Empty input (empty output)
    #[test]
    fn test_filter_changed_replaced_classes() {
        let contract1 = Felt::from_hex("0x100").unwrap();
        let contract2 = Felt::from_hex("0x200").unwrap();
        let contract3 = Felt::from_hex("0x300").unwrap();
        let contract4 = Felt::from_hex("0x400").unwrap();
        let class_a = Felt::from_hex("0xA").unwrap();
        let class_b = Felt::from_hex("0xB").unwrap();

        // Case 1: Mixed scenarios
        {
            let mut replaced = HashMap::new();
            replaced.insert(contract1, class_a); // Same as prev (filtered)
            replaced.insert(contract2, class_b); // Changed from A to B (included)
            replaced.insert(contract3, class_a); // Prev is None - new contract (included)
            replaced.insert(contract4, class_b); // Not in prev map (included)

            let mut prev = HashMap::new();
            prev.insert(contract1, Some(class_a)); // Same
            prev.insert(contract2, Some(class_a)); // Was A, now B
            prev.insert(contract3, None); // New contract

            let items = filter_changed_replaced_classes(replaced, &prev);
            let item_map: HashMap<Felt, Felt> = items.iter().map(|i| (i.contract_address, i.class_hash)).collect();

            assert_eq!(item_map.len(), 3);
            assert!(!item_map.contains_key(&contract1)); // Unchanged - filtered
            assert_eq!(item_map.get(&contract2), Some(&class_b)); // Changed
            assert_eq!(item_map.get(&contract3), Some(&class_a)); // New (prev=None)
            assert_eq!(item_map.get(&contract4), Some(&class_b)); // Missing from prev
        }

        // Case 2: Empty input
        {
            let items = filter_changed_replaced_classes(HashMap::new(), &HashMap::new());
            assert!(items.is_empty());
        }
    }
}
