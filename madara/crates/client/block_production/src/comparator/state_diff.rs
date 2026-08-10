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
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
};
use std::collections::BTreeSet;

/// Result of comparing two state diffs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDiffComparison {
    /// Both state diffs are equivalent.
    Match,
    /// State diffs differ, but only within explicitly ignored storage addresses.
    IgnoredStorageMismatch { ignored_storage_addresses: Vec<Felt> },
    /// State diffs differ. `summary` is log-safe (no addresses/hashes with full values).
    Mismatch { summary: StateDiffMismatchSummary },
}

/// Low-cardinality summary of what diverged between SD-x1 and SD-x2.
/// Intentionally omits sensitive values (no full addresses/hashes/storage values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDiffMismatchSummary {
    pub storage_diffs_match: bool,
    pub declared_classes_match: bool,
    pub deployed_contracts_match: bool,
    pub replaced_classes_match: bool,
    pub nonces_match: bool,
    /// Total number of differences across all categories.
    pub diff_count: usize,
}

impl std::fmt::Display for StateDiffMismatchSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StateDiffMismatch(storage={}, declared={}, deployed={}, replaced={}, nonces={}, total_diffs={})",
            !self.storage_diffs_match,
            !self.declared_classes_match,
            !self.deployed_contracts_match,
            !self.replaced_classes_match,
            !self.nonces_match,
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
    compare_state_diff_with_ignored_storage_addresses(sd_x1, sd_x2, &BTreeSet::new())
}

/// Compare two state diffs while allowing storage mismatches under specific contract addresses.
///
/// If the only divergence is under `ignored_storage_addresses`, the result is
/// `IgnoredStorageMismatch` instead of a fatal mismatch.
pub fn compare_state_diff_with_ignored_storage_addresses(
    sd_x1: &StateDiff,
    sd_x2: &StateDiff,
    ignored_storage_addresses: &BTreeSet<Felt>,
) -> StateDiffComparison {
    let raw_storage_x1 = sorted_storage_diffs(&sd_x1.storage_diffs);
    let raw_storage_x2 = sorted_storage_diffs(&sd_x2.storage_diffs);
    let storage_diffs_match = raw_storage_x1 == raw_storage_x2;
    let declared_classes_match =
        sorted_declared_classes(&sd_x1.declared_classes) == sorted_declared_classes(&sd_x2.declared_classes);
    let deployed_contracts_match =
        sorted_deployed_contracts(&sd_x1.deployed_contracts) == sorted_deployed_contracts(&sd_x2.deployed_contracts);
    let replaced_classes_match =
        sorted_replaced_classes(&sd_x1.replaced_classes) == sorted_replaced_classes(&sd_x2.replaced_classes);
    let nonces_match = sorted_nonces(&sd_x1.nonces) == sorted_nonces(&sd_x2.nonces);

    if storage_diffs_match
        && declared_classes_match
        && deployed_contracts_match
        && replaced_classes_match
        && nonces_match
    {
        return StateDiffComparison::Match;
    }

    let filtered_storage_match = sorted_storage_diffs_filtered(&sd_x1.storage_diffs, ignored_storage_addresses)
        == sorted_storage_diffs_filtered(&sd_x2.storage_diffs, ignored_storage_addresses);
    if !storage_diffs_match
        && filtered_storage_match
        && declared_classes_match
        && deployed_contracts_match
        && replaced_classes_match
        && nonces_match
    {
        return StateDiffComparison::IgnoredStorageMismatch {
            ignored_storage_addresses: differing_ignored_storage_addresses(
                &raw_storage_x1,
                &raw_storage_x2,
                ignored_storage_addresses,
            ),
        };
    }

    let diff_count = usize::from(!storage_diffs_match)
        + usize::from(!declared_classes_match)
        + usize::from(!deployed_contracts_match)
        + usize::from(!replaced_classes_match)
        + usize::from(!nonces_match);

    StateDiffComparison::Mismatch {
        summary: StateDiffMismatchSummary {
            storage_diffs_match,
            declared_classes_match,
            deployed_contracts_match,
            replaced_classes_match,
            nonces_match,
            diff_count,
        },
    }
}

// ── Normalization helpers ────────────────────────────────────────────────────

fn sorted_storage_diffs(diffs: &[ContractStorageDiffItem]) -> Vec<(Felt, Vec<(Felt, Felt)>)> {
    sorted_storage_diffs_filtered(diffs, &BTreeSet::new())
}

fn sorted_storage_diffs_filtered(
    diffs: &[ContractStorageDiffItem],
    ignored_storage_addresses: &BTreeSet<Felt>,
) -> Vec<(Felt, Vec<(Felt, Felt)>)> {
    let mut out: Vec<(Felt, Vec<(Felt, Felt)>)> = diffs
        .iter()
        .filter(|item| !ignored_storage_addresses.contains(&item.address))
        .map(|item| {
            let mut entries: Vec<(Felt, Felt)> = item.storage_entries.iter().map(|e| (e.key, e.value)).collect();
            entries.sort_unstable_by_key(|(k, _)| *k);
            (item.address, entries)
        })
        .collect();
    out.sort_unstable_by_key(|(addr, _)| *addr);
    out
}

fn sorted_declared_classes(classes: &[DeclaredClassItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = classes.iter().map(|c| (c.class_hash, c.compiled_class_hash)).collect();
    out.sort_unstable_by_key(|(h, _)| *h);
    out
}

fn sorted_deployed_contracts(deployed: &[DeployedContractItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = deployed.iter().map(|d| (d.address, d.class_hash)).collect();
    out.sort_unstable_by_key(|(addr, _)| *addr);
    out
}

fn sorted_replaced_classes(replaced: &[ReplacedClassItem]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = replaced.iter().map(|r| (r.contract_address, r.class_hash)).collect();
    out.sort_unstable_by_key(|(addr, _)| *addr);
    out
}

fn sorted_nonces(nonces: &[NonceUpdate]) -> Vec<(Felt, Felt)> {
    let mut out: Vec<(Felt, Felt)> = nonces.iter().map(|n| (n.contract_address, n.nonce)).collect();
    out.sort_unstable_by_key(|(addr, _)| *addr);
    out
}

fn differing_ignored_storage_addresses(
    raw_x1: &[(Felt, Vec<(Felt, Felt)>)],
    raw_x2: &[(Felt, Vec<(Felt, Felt)>)],
    ignored_storage_addresses: &BTreeSet<Felt>,
) -> Vec<Felt> {
    ignored_storage_addresses
        .iter()
        .copied()
        .filter(|address| {
            let left = raw_x1.iter().find(|(addr, _)| addr == address).map(|(_, entries)| entries);
            let right = raw_x2.iter().find(|(addr, _)| addr == address).map(|(_, entries)| entries);
            left != right
        })
        .collect()
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
    fn ignored_storage_address_mismatch_is_non_fatal() {
        let d1 = diff_with_storage(Felt::from(11u64), Felt::TWO, Felt::from(1u64));
        let d2 = diff_with_storage(Felt::from(11u64), Felt::TWO, Felt::from(2u64));
        let ignored = BTreeSet::from([Felt::from(11u64)]);

        let result = compare_state_diff_with_ignored_storage_addresses(&d1, &d2, &ignored);
        assert_eq!(
            result,
            StateDiffComparison::IgnoredStorageMismatch { ignored_storage_addresses: vec![Felt::from(11u64)] }
        );
    }

    #[test]
    fn ignored_storage_address_plus_real_mismatch_is_fatal() {
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
        let ignored = BTreeSet::from([Felt::from(11u64)]);

        let result = compare_state_diff_with_ignored_storage_addresses(&d1, &d2, &ignored);
        assert!(matches!(result, StateDiffComparison::Mismatch { .. }));
    }
}
