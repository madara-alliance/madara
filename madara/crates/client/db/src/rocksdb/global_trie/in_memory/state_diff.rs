use mp_state_update::{
    DeclaredClassCompiledClass, DeclaredClassItem, MigratedClassItem, StateDiff, TransactionStateUpdate,
};
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompiledClassUpdateOrigin {
    Declared,
    Migrated,
}

#[derive(Debug, Default)]
struct StateDiffAccumulator {
    tx_update: TransactionStateUpdate,
    class_update_origins: HashMap<Felt, CompiledClassUpdateOrigin>,
}

impl StateDiffAccumulator {
    fn apply_state_diff(&mut self, state_diff: &StateDiff) {
        let tx_update = TransactionStateUpdate::from_state_diff(state_diff);
        self.tx_update.nonces.extend(tx_update.nonces);
        self.tx_update.storage_diffs.extend(tx_update.storage_diffs);
        self.tx_update.declared_classes.extend(tx_update.declared_classes);
        self.tx_update.contract_class_hashes.extend(tx_update.contract_class_hashes);

        self.class_update_origins.extend(
            state_diff
                .declared_classes
                .iter()
                .map(|DeclaredClassItem { class_hash, .. }| (*class_hash, CompiledClassUpdateOrigin::Declared)),
        );

        for MigratedClassItem { class_hash, compiled_class_hash } in &state_diff.migrated_compiled_classes {
            self.tx_update
                .declared_classes
                .insert(*class_hash, DeclaredClassCompiledClass::Sierra(*compiled_class_hash));
            self.class_update_origins.insert(*class_hash, CompiledClassUpdateOrigin::Migrated);
        }

        for class_hash in &state_diff.old_declared_contracts {
            self.class_update_origins.remove(class_hash);
        }
    }

    fn to_state_diff(&self) -> StateDiff {
        let mut state_diff = self.tx_update.to_state_diff();

        let mut declared_classes = Vec::new();
        let mut migrated_compiled_classes = Vec::new();
        for item in state_diff.declared_classes.drain(..) {
            match self.class_update_origins.get(&item.class_hash) {
                Some(CompiledClassUpdateOrigin::Migrated) => migrated_compiled_classes.push(MigratedClassItem {
                    class_hash: item.class_hash,
                    compiled_class_hash: item.compiled_class_hash,
                }),
                _ => declared_classes.push(item),
            }
        }

        state_diff.declared_classes = declared_classes;
        state_diff.migrated_compiled_classes = migrated_compiled_classes;
        state_diff.sort();
        state_diff
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
