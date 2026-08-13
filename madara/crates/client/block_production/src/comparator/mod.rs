/// T-052: Comparator decision layer.
///
/// Orchestrates two pure comparison functions and produces a single `ComparatorDecision`
/// following the decision matrix from the design doc (§Decision Matrix):
///
/// | SD result                 | ER-x1 <= BW | ER-x1 <= ER-x2 | Result                    |
/// |--------------------------|-------------|----------------|---------------------------|
/// | match / allowed mismatch | yes         | yes            | Accept                    |
/// | match / allowed mismatch | yes         | no             | AcceptWithWarn            |
/// | match / allowed mismatch | no          | *              | StopExecutionBox(limit)   |
/// | fatal mismatch           | *           | *              | StopExecutionBox(sd_diff) |
pub mod execution_resources;
pub mod state_diff;
pub mod transaction_outputs;

use blockifier::bouncer::BouncerWeights;
use mp_state_update::StateDiff;

use crate::reexecution::ReexecExecutedTxArtifacts;

pub use execution_resources::{ExecutionResourceComparison, ResourceDeltas};
pub use state_diff::{StateDiffComparison, StateDiffMismatchSummary};
pub use transaction_outputs::{
    BlockComparisonReport, MismatchCategory, MismatchPolicy, OutputMismatchSummary, TransactionOutputComparisonConfig,
};

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
/// On `StopExecutionBox`, `bre_per_tx` carries per-tx BRE execution artifacts that
/// are paired with original rows by receipt transaction hash (C-013).
#[derive(Debug)]
pub struct CanonicalizedBlockOutput {
    pub source: CanonicalBlockSource,
    pub state_diff: StateDiff,
    pub bouncer_weights: BouncerWeights,
    /// Per-tx BRE execution artifacts (C-013). On `StopExecutionBox`, this may be a
    /// shorter canonical prefix when Blockifier reaches block capacity; the omitted
    /// speculative suffix is replayed in Blockifier-only mode.
    pub bre_per_tx: Option<Vec<ReexecExecutedTxArtifacts>>,
}

/// Reason an ExecutionBox stop was triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    OutputMismatch { summary: OutputMismatchSummary },
    StateDiffMismatch { summary: StateDiffMismatchSummary },
    ExecutionResourcesOverLimit { deltas: ResourceDeltas },
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::OutputMismatch { summary } => write!(f, "{summary}"),
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
    decide_with_report(&BlockComparisonReport::default(), sd, er)
}

/// Apply the decision matrix including strict per-transaction output mismatches.
pub fn decide_with_report(
    report: &BlockComparisonReport,
    sd: &StateDiffComparison,
    er: &ExecutionResourceComparison,
) -> ComparatorDecision {
    // Missing, extra, or duplicate transactions is the primary failure even
    // though the shortened execution will also produce an aggregate state mismatch.
    if report.has_strict_category(MismatchCategory::TransactionAlignment) {
        return ComparatorDecision::StopExecutionBox {
            reason: StopReason::OutputMismatch { summary: report.strict_summary() },
        };
    }

    // State diff mismatch is always a hard stop, regardless of resource comparison.
    if let StateDiffComparison::Mismatch { summary } = sd {
        return ComparatorDecision::StopExecutionBox {
            reason: StopReason::StateDiffMismatch { summary: summary.clone() },
        };
    }

    if report.has_strict_mismatch() {
        return ComparatorDecision::StopExecutionBox {
            reason: StopReason::OutputMismatch { summary: report.strict_summary() },
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

    fn allowed_mismatching_sd() -> StateDiffComparison {
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
        state_diff::compare_state_diff_with_allowed_fee_balance_keys(
            &d1,
            &d2,
            &BTreeSet::from([(Felt::from(11u64), Felt::ONE)]),
        )
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
    fn strict_output_mismatch_stops_before_resource_policy() {
        let mut report = BlockComparisonReport::default();
        report.push(transaction_outputs::FieldMismatch {
            category: MismatchCategory::Event,
            policy: MismatchPolicy::Strict,
            transaction_hash: Some(Felt::ONE),
            transaction_index: Some(0),
            field_path: "receipt.events[0]".into(),
            execution_box_value: "left".into(),
            blockifier_value: "right".into(),
        });

        assert!(matches!(
            decide_with_report(&report, &matching_sd(), &ok_er()),
            ComparatorDecision::StopExecutionBox { reason: StopReason::OutputMismatch { .. } }
        ));
    }

    #[test]
    fn transaction_alignment_is_primary_when_state_diff_also_mismatches() {
        let mut report = BlockComparisonReport::default();
        report.push(transaction_outputs::FieldMismatch {
            category: MismatchCategory::TransactionAlignment,
            policy: MismatchPolicy::Strict,
            transaction_hash: Some(Felt::ONE),
            transaction_index: Some(3),
            field_path: "transactions.missing_in_blockifier".into(),
            execution_box_value: "present".into(),
            blockifier_value: "missing".into(),
        });

        assert!(matches!(
            decide_with_report(&report, &mismatching_sd(), &ok_er()),
            ComparatorDecision::StopExecutionBox { reason: StopReason::OutputMismatch { .. } }
        ));
    }

    #[test]
    fn sd_match_er_warn_accepts_with_warn() {
        let result = decide(&matching_sd(), &warn_er());
        assert!(matches!(result, ComparatorDecision::AcceptWithWarn { .. }));
    }

    #[test]
    fn allowed_sd_mismatch_er_ok_accepts() {
        let result = decide(&allowed_mismatching_sd(), &ok_er());
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
