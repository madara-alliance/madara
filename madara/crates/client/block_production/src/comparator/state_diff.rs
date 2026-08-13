/// T-050: Pure state diff comparison function.
///
/// Compares two state diffs produced by ExecutionBox (SD-x1) and Blockifier re-execution (SD-x2).
/// A mismatch is a hard stop condition for ExecutionBox (design doc §State Diff Rule).
///
/// Requirements (from design):
/// - deterministic and side-effect free
/// - no logging/metrics inside this pure function
/// - mismatch summary is low-cardinality/log-safe
///
/// TODO (deferred): lock canonicalization/normalization spec before equality comparison
/// (ordering normalization and any representation-level normalization).
use mp_convert::Felt;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff,
};
use std::collections::{BTreeMap, BTreeSet};

pub type StorageAddressKey = (Felt, Felt);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedStorageValueMismatch {
    pub contract_address: Felt,
    pub storage_key: Felt,
    pub execution_box_value: Felt,
    pub blockifier_value: Felt,
}

/// Result of comparing two state diffs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDiffComparison {
    /// Both state diffs are equivalent.
    Match,
    /// State diffs differ only in values at derived sender/sequencer fee-balance keys.
    AllowedFeeBalanceMismatch { mismatches: Vec<AllowedStorageValueMismatch> },
    /// State diffs differ. `summary` is log-safe (no addresses/hashes with full values).
    Mismatch { summary: StateDiffMismatchSummary },
}

/// Low-cardinality summary of what diverged between SD-x1 and SD-x2.
/// Intentionally omits sensitive values (no full addresses/hashes/storage values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDiffMismatchSummary {
    pub storage_diffs_match: bool,
    pub declared_classes_match: bool,
    pub old_declared_classes_match: bool,
    pub deployed_contracts_match: bool,
    pub replaced_classes_match: bool,
    pub nonces_match: bool,
    pub migrated_compiled_classes_match: bool,
    /// Total number of differences across all categories.
    pub diff_count: usize,
}

impl std::fmt::Display for StateDiffMismatchSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StateDiffMismatch(storage={}, declared={}, deprecated_declared={}, deployed={}, replaced={}, nonces={}, migrated={}, total_diffs={})",
            !self.storage_diffs_match,
            !self.declared_classes_match,
            !self.old_declared_classes_match,
            !self.deployed_contracts_match,
            !self.replaced_classes_match,
            !self.nonces_match,
            !self.migrated_compiled_classes_match,
            self.diff_count,
        )
    }
}

/// Compare two state diffs for equivalence.
///
/// Normalizes each diff by sorting all entries before comparing, so ordering
/// differences from execution do not cause false mismatches.
#[allow(dead_code)]
pub fn compare_state_diff(sd_x1: &StateDiff, sd_x2: &StateDiff) -> StateDiffComparison {
    compare_state_diff_with_allowed_fee_balance_keys(sd_x1, sd_x2, &BTreeSet::new())
}

/// Compare two state diffs while allowing value-only differences at specific fee-balance keys.
///
/// The complete storage key set must remain identical. A missing or extra key is fatal even
/// when that key is approved. Only values may differ at `allowed_fee_balance_keys`.
pub fn compare_state_diff_with_allowed_fee_balance_keys(
    sd_x1: &StateDiff,
    sd_x2: &StateDiff,
    allowed_fee_balance_keys: &BTreeSet<StorageAddressKey>,
) -> StateDiffComparison {
    let raw_storage_x1 = storage_map(&sd_x1.storage_diffs);
    let raw_storage_x2 = storage_map(&sd_x2.storage_diffs);
    let storage_exact_match = raw_storage_x1 == raw_storage_x2;
    let storage_key_sets_match = raw_storage_x1.keys().eq(raw_storage_x2.keys());
    let allowed_storage_mismatches = if storage_key_sets_match {
        raw_storage_x1
            .iter()
            .filter_map(|(&(contract_address, storage_key), execution_box_values)| {
                let blockifier_values = &raw_storage_x2[&(contract_address, storage_key)];
                (execution_box_values.len() == 1
                    && blockifier_values.len() == 1
                    && execution_box_values != blockifier_values
                    && allowed_fee_balance_keys.contains(&(contract_address, storage_key)))
                .then_some(AllowedStorageValueMismatch {
                    contract_address,
                    storage_key,
                    execution_box_value: execution_box_values[0],
                    blockifier_value: blockifier_values[0],
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let storage_diffs_match = storage_key_sets_match
        && raw_storage_x1.iter().all(|(key, values)| {
            raw_storage_x2.get(key).is_some_and(|other| {
                values == other || (values.len() == 1 && other.len() == 1 && allowed_fee_balance_keys.contains(key))
            })
        });
    let declared_classes_match =
        sorted_declared_classes(&sd_x1.declared_classes) == sorted_declared_classes(&sd_x2.declared_classes);
    let old_declared_classes_match =
        sorted_felts(&sd_x1.old_declared_contracts) == sorted_felts(&sd_x2.old_declared_contracts);
    let deployed_contracts_match =
        sorted_deployed_contracts(&sd_x1.deployed_contracts) == sorted_deployed_contracts(&sd_x2.deployed_contracts);
    let replaced_classes_match =
        sorted_replaced_classes(&sd_x1.replaced_classes) == sorted_replaced_classes(&sd_x2.replaced_classes);
    let nonces_match = sorted_nonces(&sd_x1.nonces) == sorted_nonces(&sd_x2.nonces);
    let migrated_compiled_classes_match = sorted_migrated_classes(&sd_x1.migrated_compiled_classes)
        == sorted_migrated_classes(&sd_x2.migrated_compiled_classes);

    if storage_exact_match
        && declared_classes_match
        && old_declared_classes_match
        && deployed_contracts_match
        && replaced_classes_match
        && nonces_match
        && migrated_compiled_classes_match
    {
        return StateDiffComparison::Match;
    }

    if storage_diffs_match
        && !allowed_storage_mismatches.is_empty()
        && declared_classes_match
        && old_declared_classes_match
        && deployed_contracts_match
        && replaced_classes_match
        && nonces_match
        && migrated_compiled_classes_match
    {
        return StateDiffComparison::AllowedFeeBalanceMismatch { mismatches: allowed_storage_mismatches };
    }

    let diff_count = usize::from(!storage_diffs_match)
        + usize::from(!declared_classes_match)
        + usize::from(!old_declared_classes_match)
        + usize::from(!deployed_contracts_match)
        + usize::from(!replaced_classes_match)
        + usize::from(!nonces_match)
        + usize::from(!migrated_compiled_classes_match);

    StateDiffComparison::Mismatch {
        summary: StateDiffMismatchSummary {
            storage_diffs_match,
            declared_classes_match,
            old_declared_classes_match,
            deployed_contracts_match,
            replaced_classes_match,
            nonces_match,
            migrated_compiled_classes_match,
            diff_count,
        },
    }
}

// ── Normalization helpers ────────────────────────────────────────────────────

fn storage_map(diffs: &[ContractStorageDiffItem]) -> BTreeMap<StorageAddressKey, Vec<Felt>> {
    let mut out = BTreeMap::new();
    for item in diffs {
        for entry in &item.storage_entries {
            out.entry((item.address, entry.key)).or_insert_with(Vec::new).push(entry.value);
        }
    }
    for values in out.values_mut() {
        values.sort_unstable();
    }
    out
}

fn sorted_declared_classes(classes: &[DeclaredClassItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = classes.iter().map(|c| (c.class_hash, c.compiled_class_hash)).collect();
    out.sort_unstable();
    out
}

fn sorted_felts(values: &[Felt]) -> Vec<Felt> {
    let mut out = values.to_vec();
    out.sort_unstable();
    out
}

fn sorted_migrated_classes(classes: &[MigratedClassItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<_> = classes.iter().map(|item| (item.class_hash, item.compiled_class_hash)).collect();
    out.sort_unstable();
    out
}

fn sorted_deployed_contracts(deployed: &[DeployedContractItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = deployed.iter().map(|d| (d.address, d.class_hash)).collect();
    out.sort_unstable();
    out
}

fn sorted_replaced_classes(replaced: &[ReplacedClassItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = replaced.iter().map(|r| (r.contract_address, r.class_hash)).collect();
    out.sort_unstable();
    out
}

fn sorted_nonces(nonces: &[NonceUpdate]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = nonces.iter().map(|n| (n.contract_address, n.nonce)).collect();
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_state_update::{ContractStorageDiffItem, StorageEntry};

    fn empty_diff() -> StateDiff {
        StateDiff::default()
    }

    fn diff_with_storage(addr: Felt, key: Felt, val: Felt) -> StateDiff {
        StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: addr,
                storage_entries: vec![StorageEntry { key, value: val }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn identical_empty_diffs_match() {
        assert_eq!(compare_state_diff(&empty_diff(), &empty_diff()), StateDiffComparison::Match);
    }

    #[test]
    fn identical_storage_diffs_match() {
        let d = diff_with_storage(Felt::ONE, Felt::TWO, Felt::from(42u64));
        assert_eq!(compare_state_diff(&d, &d), StateDiffComparison::Match);
    }

    #[test]
    fn different_storage_value_is_mismatch() {
        let d1 = diff_with_storage(Felt::ONE, Felt::TWO, Felt::from(1u64));
        let d2 = diff_with_storage(Felt::ONE, Felt::TWO, Felt::from(2u64));
        let result = compare_state_diff(&d1, &d2);
        assert!(matches!(result, StateDiffComparison::Mismatch { .. }));
        if let StateDiffComparison::Mismatch { summary } = result {
            assert!(!summary.storage_diffs_match);
            assert!(summary.declared_classes_match);
        }
    }

    #[test]
    fn ordering_difference_does_not_cause_mismatch() {
        // Same two storage entries but in different order.
        let d1 = StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::from(10u64) }],
                },
                ContractStorageDiffItem {
                    address: Felt::from(2u64),
                    storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(20u64) }],
                },
            ],
            ..Default::default()
        };
        let d2 = StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::from(2u64),
                    storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(20u64) }],
                },
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::from(10u64) }],
                },
            ],
            ..Default::default()
        };
        assert_eq!(compare_state_diff(&d1, &d2), StateDiffComparison::Match);
    }

    #[test]
    fn approved_fee_balance_value_mismatch_is_allowed() {
        let d1 = diff_with_storage(Felt::from(11u64), Felt::TWO, Felt::from(1u64));
        let d2 = diff_with_storage(Felt::from(11u64), Felt::TWO, Felt::from(2u64));
        let allowed = BTreeSet::from([(Felt::from(11u64), Felt::TWO)]);

        let result = compare_state_diff_with_allowed_fee_balance_keys(&d1, &d2, &allowed);
        assert!(matches!(result, StateDiffComparison::AllowedFeeBalanceMismatch { .. }));
    }

    #[test]
    fn approved_fee_balance_plus_unrelated_mismatch_is_fatal() {
        let d1 = StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::from(11u64),
                    storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::from(1u64) }],
                },
                ContractStorageDiffItem {
                    address: Felt::from(22u64),
                    storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::from(3u64) }],
                },
            ],
            ..Default::default()
        };
        let d2 = StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::from(11u64),
                    storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::from(2u64) }],
                },
                ContractStorageDiffItem {
                    address: Felt::from(22u64),
                    storage_entries: vec![StorageEntry { key: Felt::TWO, value: Felt::from(4u64) }],
                },
            ],
            ..Default::default()
        };
        let allowed = BTreeSet::from([(Felt::from(11u64), Felt::TWO)]);

        let result = compare_state_diff_with_allowed_fee_balance_keys(&d1, &d2, &allowed);
        assert!(matches!(result, StateDiffComparison::Mismatch { .. }));
    }

    #[test]
    fn approved_fee_balance_key_must_exist_on_both_sides() {
        let d1 = diff_with_storage(Felt::from(11u64), Felt::TWO, Felt::from(1u64));
        let allowed = BTreeSet::from([(Felt::from(11u64), Felt::TWO)]);

        let result = compare_state_diff_with_allowed_fee_balance_keys(&d1, &StateDiff::default(), &allowed);
        assert!(matches!(result, StateDiffComparison::Mismatch { .. }));
    }

    #[test]
    fn duplicate_approved_fee_balance_key_is_strict() {
        let d1 = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: Felt::from(11u64),
                storage_entries: vec![
                    StorageEntry { key: Felt::TWO, value: Felt::from(1u64) },
                    StorageEntry { key: Felt::TWO, value: Felt::from(2u64) },
                ],
            }],
            ..Default::default()
        };
        let d2 = diff_with_storage(Felt::from(11u64), Felt::TWO, Felt::from(2u64));
        let allowed = BTreeSet::from([(Felt::from(11u64), Felt::TWO)]);

        assert!(matches!(
            compare_state_diff_with_allowed_fee_balance_keys(&d1, &d2, &allowed),
            StateDiffComparison::Mismatch { .. }
        ));
    }

    #[test]
    fn deprecated_and_migrated_classes_are_strict() {
        let old_declared = StateDiff { old_declared_contracts: vec![Felt::ONE], ..Default::default() };
        let migrated = StateDiff {
            migrated_compiled_classes: vec![MigratedClassItem {
                class_hash: Felt::ONE,
                compiled_class_hash: Felt::TWO,
            }],
            ..Default::default()
        };

        for result in [
            compare_state_diff(&old_declared, &StateDiff::default()),
            compare_state_diff(&migrated, &StateDiff::default()),
        ] {
            assert!(matches!(result, StateDiffComparison::Mismatch { .. }));
        }
    }
}
