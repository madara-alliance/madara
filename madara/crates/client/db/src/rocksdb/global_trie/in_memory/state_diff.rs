use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use starknet_types_core::felt::Felt;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
struct StateDiffAccumulator {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    contract_class_updates: HashMap<Felt, (Felt, bool)>, // (class_hash, from_deploy)
    class_hash_updates: HashMap<Felt, (Felt, bool)>,     // (compiled_class_hash, migrated)
    old_declared_classes: HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
}

impl StateDiffAccumulator {
    fn apply_state_diff(&mut self, state_diff: &StateDiff) {
        tracing::info!(
            "parallel_state_diff_apply input storage_diff_contracts={} storage_entries={} deployed_contracts={} replaced_classes={} declared_classes={} migrated_compiled_classes={} nonces={} old_declared_contracts={}",
            state_diff.storage_diffs.len(),
            state_diff.storage_diffs.iter().map(|item| item.storage_entries.len()).sum::<usize>(),
            state_diff.deployed_contracts.len(),
            state_diff.replaced_classes.len(),
            state_diff.declared_classes.len(),
            state_diff.migrated_compiled_classes.len(),
            state_diff.nonces.len(),
            state_diff.old_declared_contracts.len()
        );

        for ContractStorageDiffItem { address, storage_entries } in &state_diff.storage_diffs {
            tracing::info!(
                "parallel_state_diff_apply_storage contract_address={:#x} storage_entry_count={} storage_entries={:?}",
                address,
                storage_entries.len(),
                storage_entries.iter().map(|entry| format!("{:#x}={:#x}", entry.key, entry.value)).collect::<Vec<_>>()
            );
        }
        for DeployedContractItem { address, class_hash } in &state_diff.deployed_contracts {
            tracing::info!(
                "parallel_state_diff_apply_deploy contract_address={:#x} class_hash={:#x}",
                address,
                class_hash
            );
        }
        for ReplacedClassItem { contract_address, class_hash } in &state_diff.replaced_classes {
            tracing::info!(
                "parallel_state_diff_apply_replace contract_address={:#x} class_hash={:#x}",
                contract_address,
                class_hash
            );
        }
        for DeclaredClassItem { class_hash, compiled_class_hash } in &state_diff.declared_classes {
            tracing::info!(
                "parallel_state_diff_apply_declared class_hash={:#x} compiled_class_hash={:#x}",
                class_hash,
                compiled_class_hash
            );
        }
        for MigratedClassItem { class_hash, compiled_class_hash } in &state_diff.migrated_compiled_classes {
            tracing::info!(
                "parallel_state_diff_apply_migrated class_hash={:#x} compiled_class_hash={:#x}",
                class_hash,
                compiled_class_hash
            );
        }
        for NonceUpdate { contract_address, nonce } in &state_diff.nonces {
            tracing::info!(
                "parallel_state_diff_apply_nonce contract_address={:#x} nonce={:#x}",
                contract_address,
                nonce
            );
        }

        for ContractStorageDiffItem { address, storage_entries } in &state_diff.storage_diffs {
            let storage = self.storage_diffs.entry(*address).or_default();
            for StorageEntry { key, value } in storage_entries {
                storage.insert(*key, *value);
            }
        }

        for DeployedContractItem { address, class_hash } in &state_diff.deployed_contracts {
            self.contract_class_updates.insert(*address, (*class_hash, true));
        }
        for ReplacedClassItem { contract_address, class_hash } in &state_diff.replaced_classes {
            self.contract_class_updates.insert(*contract_address, (*class_hash, false));
        }

        for DeclaredClassItem { class_hash, compiled_class_hash } in &state_diff.declared_classes {
            self.class_hash_updates.insert(*class_hash, (*compiled_class_hash, false));
        }
        for MigratedClassItem { class_hash, compiled_class_hash } in &state_diff.migrated_compiled_classes {
            self.class_hash_updates.insert(*class_hash, (*compiled_class_hash, true));
        }

        self.old_declared_classes.extend(state_diff.old_declared_contracts.iter().copied());
        for NonceUpdate { contract_address, nonce } in &state_diff.nonces {
            self.nonces.insert(*contract_address, *nonce);
        }
    }

    fn to_state_diff(&self) -> StateDiff {
        let mut storage_diffs: Vec<_> = self
            .storage_diffs
            .iter()
            .map(|(address, entries)| {
                let mut storage_entries: Vec<_> =
                    entries.iter().map(|(key, value)| StorageEntry { key: *key, value: *value }).collect();
                storage_entries.sort_by_key(|entry| entry.key);
                ContractStorageDiffItem { address: *address, storage_entries }
            })
            .collect();
        storage_diffs.sort_by_key(|entry| entry.address);

        let mut deployed_contracts = Vec::new();
        let mut replaced_classes = Vec::new();
        for (address, (class_hash, from_deploy)) in self.contract_class_updates.iter() {
            if *from_deploy {
                deployed_contracts.push(DeployedContractItem { address: *address, class_hash: *class_hash });
            } else {
                replaced_classes.push(ReplacedClassItem { contract_address: *address, class_hash: *class_hash });
            }
        }
        deployed_contracts.sort_by_key(|item| item.address);
        replaced_classes.sort_by_key(|item| item.contract_address);

        let mut declared_classes = Vec::new();
        let mut migrated_compiled_classes = Vec::new();
        for (class_hash, (compiled_class_hash, migrated)) in self.class_hash_updates.iter() {
            if *migrated {
                migrated_compiled_classes
                    .push(MigratedClassItem { class_hash: *class_hash, compiled_class_hash: *compiled_class_hash });
            } else {
                declared_classes
                    .push(DeclaredClassItem { class_hash: *class_hash, compiled_class_hash: *compiled_class_hash });
            }
        }
        declared_classes.sort_by_key(|item| item.class_hash);
        migrated_compiled_classes.sort_by_key(|item| item.class_hash);

        let mut nonces: Vec<_> = self
            .nonces
            .iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address: *contract_address, nonce: *nonce })
            .collect();
        nonces.sort_by_key(|item| item.contract_address);

        let mut old_declared_contracts: Vec<_> = self.old_declared_classes.iter().copied().collect();
        old_declared_contracts.sort();

        let squashed = StateDiff {
            storage_diffs,
            old_declared_contracts,
            declared_classes,
            deployed_contracts,
            replaced_classes,
            nonces,
            migrated_compiled_classes,
        };

        tracing::info!(
            "parallel_state_diff_squashed output storage_diff_contracts={} storage_entries={} deployed_contracts={} replaced_classes={} declared_classes={} migrated_compiled_classes={} nonces={} old_declared_contracts={}",
            squashed.storage_diffs.len(),
            squashed.storage_diffs.iter().map(|item| item.storage_entries.len()).sum::<usize>(),
            squashed.deployed_contracts.len(),
            squashed.replaced_classes.len(),
            squashed.declared_classes.len(),
            squashed.migrated_compiled_classes.len(),
            squashed.nonces.len(),
            squashed.old_declared_contracts.len()
        );
        for ContractStorageDiffItem { address, storage_entries } in &squashed.storage_diffs {
            tracing::info!(
                "parallel_state_diff_squashed_storage contract_address={:#x} storage_entry_count={} storage_entries={:?}",
                address,
                storage_entries.len(),
                storage_entries
                    .iter()
                    .map(|entry| format!("{:#x}={:#x}", entry.key, entry.value))
                    .collect::<Vec<_>>()
            );
        }
        for DeployedContractItem { address, class_hash } in &squashed.deployed_contracts {
            tracing::info!(
                "parallel_state_diff_squashed_deploy contract_address={:#x} class_hash={:#x}",
                address,
                class_hash
            );
        }
        for ReplacedClassItem { contract_address, class_hash } in &squashed.replaced_classes {
            tracing::info!(
                "parallel_state_diff_squashed_replace contract_address={:#x} class_hash={:#x}",
                contract_address,
                class_hash
            );
        }
        for DeclaredClassItem { class_hash, compiled_class_hash } in &squashed.declared_classes {
            tracing::info!(
                "parallel_state_diff_squashed_declared class_hash={:#x} compiled_class_hash={:#x}",
                class_hash,
                compiled_class_hash
            );
        }
        for MigratedClassItem { class_hash, compiled_class_hash } in &squashed.migrated_compiled_classes {
            tracing::info!(
                "parallel_state_diff_squashed_migrated class_hash={:#x} compiled_class_hash={:#x}",
                class_hash,
                compiled_class_hash
            );
        }
        for NonceUpdate { contract_address, nonce } in &squashed.nonces {
            tracing::info!(
                "parallel_state_diff_squashed_nonce contract_address={:#x} nonce={:#x}",
                contract_address,
                nonce
            );
        }

        squashed
    }
}

pub fn squash_state_diffs<'a>(state_diffs: impl IntoIterator<Item = &'a StateDiff>) -> StateDiff {
    let mut accumulator = StateDiffAccumulator::default();
    for state_diff in state_diffs {
        accumulator.apply_state_diff(state_diff);
    }
    accumulator.to_state_diff()
}

pub fn cumulative_squashed_state_diffs<'a>(state_diffs: impl IntoIterator<Item = &'a StateDiff>) -> Vec<StateDiff> {
    let mut accumulator = StateDiffAccumulator::default();
    let mut out = Vec::new();
    for state_diff in state_diffs {
        accumulator.apply_state_diff(state_diff);
        out.push(accumulator.to_state_diff());
    }
    out
}
