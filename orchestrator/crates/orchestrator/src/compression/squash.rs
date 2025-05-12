use crate::error::job::JobError;
use crate::error::other::OtherError;
use color_eyre::eyre::eyre;
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem, StateDiff,
    StateUpdate, StorageEntry,
};
use std::collections::{HashMap, HashSet};

/// squash_state_updates merge all the StateUpdate into a single StateUpdate
pub fn squash_state_updates(state_updates: Vec<StateUpdate>) -> Result<StateUpdate, JobError> {
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

    // Storage diffs
    for (contract_addr, storage_map) in storage_diffs_map {
        let storage_entries = storage_map
            .into_iter()
            .filter(|(_, value)| *value != Felt::ZERO) // Filter out entries with value 0x0
            .map(|(key, value)| StorageEntry { key, value })
            .collect::<Vec<_>>();

        // Only include contracts that have non-zero storage entries
        if !storage_entries.is_empty() {
            let contract_storage_diff = ContractStorageDiffItem { address: contract_addr, storage_entries };

            state_diff.storage_diffs.push(contract_storage_diff);
        }
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
