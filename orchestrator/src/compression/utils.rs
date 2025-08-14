use starknet_core::types::StateUpdate;

pub fn sort_state_diff(state_diff: &mut StateUpdate) {
    // Sort storage diffs
    state_diff.state_diff.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
    for diff in &mut state_diff.state_diff.storage_diffs {
        diff.storage_entries.sort_by(|a, b| a.key.cmp(&b.key));
    }

    // Sort the rest
    state_diff.state_diff.deprecated_declared_classes.sort();
    state_diff.state_diff.declared_classes.sort_by(|a, b| a.class_hash.cmp(&b.class_hash));
    state_diff.state_diff.deployed_contracts.sort_by(|a, b| a.address.cmp(&b.address));
    state_diff.state_diff.replaced_classes.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
    state_diff.state_diff.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
}
