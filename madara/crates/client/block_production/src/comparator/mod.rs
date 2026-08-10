/// T-052: Comparator decision layer.
///
/// Orchestrates two pure comparison functions and produces a single `ComparatorDecision`
/// following the decision matrix from the design doc (§Decision Matrix):
///
/// | SD result                 | ER-x1 <= BW | ER-x1 <= ER-x2 | Result                    |
/// |--------------------------|-------------|----------------|---------------------------|
/// | match / ignored mismatch | yes         | yes            | Accept                    |
/// | match / ignored mismatch | yes         | no             | AcceptWithWarn            |
/// | match / ignored mismatch | no          | *              | StopExecutionBox(limit)   |
/// | fatal mismatch           | *           | *              | StopExecutionBox(sd_diff) |
pub mod execution_resources;
pub mod state_diff;

use blockifier::bouncer::BouncerWeights;
use mp_state_update::StateDiff;

use crate::reexecution::ReexecExecutedTxArtifacts;

pub use execution_resources::{ExecutionResourceComparison, ResourceDeltas};
pub use state_diff::{StateDiffComparison, StateDiffMismatchSummary};

/// Which execution pipeline produced the canonical output for a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalBlockSource {
    /// ExecutionBox output (outputs-1) — used on Accept / AcceptWithWarn.
    ExecutionBox,
    /// Blockifier re-execution output (outputs-2) — used on StopExecutionBox.
    BlockifierReexec,
}

/// Canonical block output selected by the comparator for block X.
///
/// Contains the state diff and bouncer weights that the close pipeline must use.
/// On `StopExecutionBox`, `bre_per_tx` carries ordered per-tx BRE execution artifacts
/// so that external preconfirmed content can be rebuilt from BRE (C-013).
#[derive(Debug)]
pub struct CanonicalizedBlockOutput {
    pub source: CanonicalBlockSource,
    pub state_diff: StateDiff,
    pub bouncer_weights: BouncerWeights,
    /// Per-tx BRE execution artifacts (C-013). Present on `StopExecutionBox` when the
    /// worker produced complete per-tx results. `None` on Accept/AcceptWithWarn (EB-backed
    /// promotion used instead) or when BRE per-tx collection was incomplete.
    pub bre_per_tx: Option<Vec<ReexecExecutedTxArtifacts>>,
}

/// Reason an ExecutionBox stop was triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    StateDiffMismatch { summary: StateDiffMismatchSummary },
    ExecutionResourcesOverLimit { deltas: ResourceDeltas },
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::StateDiffMismatch { summary } => write!(f, "StateDiffMismatch({summary})"),
            StopReason::ExecutionResourcesOverLimit { deltas } => {
                write!(f, "ExecutionResourcesOverLimit({deltas})")
            }
        }
    }
}

/// Final decision produced by the comparator for a single block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComparatorDecision {
    /// Block comparison passed. ExecutionBox output (SD-x1, ER-x1) is canonical.
    Accept,
    /// Block accepted, but ER-x1 > ER-x2 while still within block limits.
    /// ExecutionBox stays active; caller must emit WARN and increment metric.
    AcceptWithWarn { resource_deltas: ResourceDeltas },
    /// Fatal mismatch. Caller must stop ExecutionBox and trigger step-6 fallback.
    StopExecutionBox { reason: StopReason },
}

/// Apply the decision matrix to pre-computed comparison results.
///
/// No I/O, no side-effects. Logging and metrics are the caller's responsibility.
pub fn decide(sd: &StateDiffComparison, er: &ExecutionResourceComparison) -> ComparatorDecision {
    // State diff mismatch is always a hard stop, regardless of resource comparison.
    if let StateDiffComparison::Mismatch { summary } = sd {
        return ComparatorDecision::StopExecutionBox {
            reason: StopReason::StateDiffMismatch { summary: summary.clone() },
        };
    }

    // State diffs match; check execution resources.
    match er {
        ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { deltas } => {
            ComparatorDecision::StopExecutionBox {
                reason: StopReason::ExecutionResourcesOverLimit { deltas: deltas.clone() },
            }
        }
        ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { deltas } => {
            ComparatorDecision::AcceptWithWarn { resource_deltas: deltas.clone() }
        }
        ExecutionResourceComparison::Ok => ComparatorDecision::Accept,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_convert::Felt;
    use mp_state_update::StateDiff;
    use std::collections::BTreeSet;

    fn matching_sd() -> StateDiffComparison {
        state_diff::compare_state_diff(&StateDiff::default(), &StateDiff::default())
    }

    fn mismatching_sd() -> StateDiffComparison {
        use mp_state_update::{ContractStorageDiffItem, StorageEntry};
        let d1 = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: Felt::ONE,
                storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(1u64) }],
            }],
            ..Default::default()
        };
        state_diff::compare_state_diff(&d1, &StateDiff::default())
    }

    fn ignored_mismatching_sd() -> StateDiffComparison {
        use mp_state_update::{ContractStorageDiffItem, StorageEntry};
        let d1 = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: Felt::from(11u64),
                storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(1u64) }],
            }],
            ..Default::default()
        };
        let d2 = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: Felt::from(11u64),
                storage_entries: vec![StorageEntry { key: Felt::ONE, value: Felt::from(2u64) }],
            }],
            ..Default::default()
        };
        state_diff::compare_state_diff_with_ignored_storage_addresses(&d1, &d2, &BTreeSet::from([Felt::from(11u64)]))
    }

    fn ok_er() -> ExecutionResourceComparison {
        ExecutionResourceComparison::Ok
    }

    fn warn_er() -> ExecutionResourceComparison {
        ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec {
            deltas: ResourceDeltas { l1_gas: Some(10), ..Default::default() },
        }
    }

    fn fatal_er() -> ExecutionResourceComparison {
        ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit {
            deltas: ResourceDeltas { sierra_gas: Some(999), ..Default::default() },
        }
    }

    // Decision matrix tests.

    #[test]
    fn sd_match_er_ok_accepts() {
        assert_eq!(decide(&matching_sd(), &ok_er()), ComparatorDecision::Accept);
    }

    #[test]
    fn sd_match_er_warn_accepts_with_warn() {
        let result = decide(&matching_sd(), &warn_er());
        assert!(matches!(result, ComparatorDecision::AcceptWithWarn { .. }));
    }

    #[test]
    fn ignored_sd_mismatch_er_ok_accepts() {
        let result = decide(&ignored_mismatching_sd(), &ok_er());
        assert_eq!(result, ComparatorDecision::Accept);
    }

    #[test]
    fn sd_match_er_fatal_stops() {
        let result = decide(&matching_sd(), &fatal_er());
        assert!(matches!(
            result,
            ComparatorDecision::StopExecutionBox { reason: StopReason::ExecutionResourcesOverLimit { .. } }
        ));
    }

    #[test]
    fn sd_mismatch_always_stops_regardless_of_er() {
        for er in [ok_er(), warn_er(), fatal_er()] {
            let result = decide(&mismatching_sd(), &er);
            assert!(
                matches!(result, ComparatorDecision::StopExecutionBox { reason: StopReason::StateDiffMismatch { .. } }),
                "expected StopExecutionBox(StateDiffMismatch) for er={er:?}"
            );
        }
    }
}
