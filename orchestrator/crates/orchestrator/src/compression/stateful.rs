use color_eyre::Result;
use starknet::core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem,
    StateUpdate, StorageEntry,
};
use std::collections::HashMap;

const SPECIAL_ADDRESS: &str = "0x2";
const GLOBAL_COUNTER_SLOT: &str = "0x0";

/// Represents a mapping from one value to another
#[derive(Debug)]
struct ValueMapping {
    mappings: HashMap<Felt, Felt>,
}

impl ValueMapping {
    /// Creates a new ValueMapping from storage entries at the special address
    fn from_special_address(state_update: &StateUpdate) -> Self {
        let mut mappings = HashMap::new();

        // Find the special address storage entries
        if let Some(special_contract) = state_update
            .state_diff
            .storage_diffs
            .iter()
            .find(|diff| diff.address == Felt::from_hex(SPECIAL_ADDRESS).unwrap())
        {
            // Add each key-value pair to our mapping, ignoring the global counter-slot
            for entry in &special_contract.storage_entries {
                if entry.key != Felt::from_hex(GLOBAL_COUNTER_SLOT).unwrap() {
                    mappings.insert(entry.key, entry.value);
                }
            }
        }

        ValueMapping { mappings }
    }

    /// Maps a value using the stored mappings
    fn map_value(&self, value: &Felt) -> Felt {
        self.mappings.get(value).cloned().unwrap_or(*value)
    }
}

/// Compresses a state update using stateful compression
///
/// This function:
/// 1. Extracts mapping information from the special address (0x2)
/// 2. Uses this mapping to transform other addresses and values
/// 3. Preserves the special address entries as-is
///
/// # Arguments
/// * `json_str` - JSON string containing the state update to compress
///
/// # Returns
/// A compressed StateUpdate with values mapped according to the special address mappings
pub fn compress(state_update: &StateUpdate) -> Result<StateUpdate> {
    let mut state_update = state_update.clone();

    // Create mapping from the special address
    let mapping = ValueMapping::from_special_address(&state_update);

    // Create a new storage diffs vector
    let mut new_storage_diffs = Vec::new();

    // Process each contract's storage diffs
    for diff in state_update.state_diff.storage_diffs {
        if diff.address == Felt::from_hex(SPECIAL_ADDRESS)? {
            // Preserve special address entries as-is
            new_storage_diffs.push(diff);
            continue;
        }

        // Map the contract address
        let mapped_address = mapping.map_value(&diff.address);

        // Map storage entries
        let mapped_entries: Vec<StorageEntry> = diff
            .storage_entries
            .into_iter()
            .map(|entry| StorageEntry { key: mapping.map_value(&entry.key), value: entry.value })
            .collect();

        // Create a new storage diff with mapped values
        new_storage_diffs.push(ContractStorageDiffItem { address: mapped_address, storage_entries: mapped_entries });
    }

    // Update storage diffs in state update
    state_update.state_diff.storage_diffs = new_storage_diffs;

    // Map other fields that need mapping
    state_update.state_diff.deployed_contracts = state_update
        .state_diff
        .deployed_contracts
        .into_iter()
        .map(|item| DeployedContractItem {
            address: mapping.map_value(&item.address),
            class_hash: mapping.map_value(&item.class_hash),
        })
        .collect();

    state_update.state_diff.declared_classes = state_update
        .state_diff
        .declared_classes
        .into_iter()
        .map(|item| DeclaredClassItem {
            class_hash: mapping.map_value(&item.class_hash),
            compiled_class_hash: mapping.map_value(&item.compiled_class_hash),
        })
        .collect();

    // Map nonces
    state_update.state_diff.nonces = state_update
        .state_diff
        .nonces
        .into_iter()
        .map(|item| NonceUpdate {
            contract_address: mapping.map_value(&item.contract_address),
            nonce: mapping.map_value(&item.nonce),
        })
        .collect();

    // Map replaced classes
    state_update.state_diff.replaced_classes = state_update
        .state_diff
        .replaced_classes
        .into_iter()
        .map(|item| ReplacedClassItem {
            contract_address: mapping.map_value(&item.contract_address),
            class_hash: mapping.map_value(&item.class_hash),
        })
        .collect();

    // Map deprecated declared classes
    state_update.state_diff.deprecated_declared_classes = state_update
        .state_diff
        .deprecated_declared_classes
        .into_iter()
        .map(|class_hash| mapping.map_value(&class_hash))
        .collect();

    // Map block hashes and roots if needed
    state_update.block_hash = mapping.map_value(&state_update.block_hash);
    state_update.new_root = mapping.map_value(&state_update.new_root);
    state_update.old_root = mapping.map_value(&state_update.old_root);

    Ok(state_update)
}
