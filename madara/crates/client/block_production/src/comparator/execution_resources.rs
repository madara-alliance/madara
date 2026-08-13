/// T-051: Pure execution resource comparison function.
///
/// Compares execution resources produced by ExecutionBox (ER-x1) and Blockifier re-execution
/// (ER-x2) against the block capacity limit (BW).
///
/// Rules (from design doc §Execution Resource Rules):
/// - `ER-x1 > BW`        → FatalExecutionBoxGreaterThanBlockLimit (hard stop)
/// - `ER-x1 > ER-x2` and `ER-x1 <= BW` → WarnExecutionBoxGreaterThanReexec (warn only)
/// - otherwise           → Ok
///
/// Requirements (from design):
/// - deterministic and side-effect free
/// - no logging/metrics inside this pure function
use blockifier::bouncer::BouncerWeights;
use starknet_api::execution_resources::GasAmount;

/// Result of comparing execution resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionResourceComparison {
    /// ER-x1 is within block limits and not greater than ER-x2. All clear.
    Ok,
    /// ER-x1 > ER-x2 but ER-x1 <= BW. Warn-only; does not stop ExecutionBox.
    WarnExecutionBoxGreaterThanReexec { deltas: ResourceDeltas },
    /// ER-x1 > BW. Hard stop condition for ExecutionBox.
    FatalExecutionBoxGreaterThanBlockLimit { deltas: ResourceDeltas },
}

/// Per-field excess deltas (er_x1 - limit or er_x1 - er_x2, where positive).
/// All fields are Option<u64> — Some(delta) when that field exceeds, None when within limits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceDeltas {
    pub l1_gas: Option<u64>,
    pub message_segment_length: Option<u64>,
    pub n_events: Option<u64>,
    pub state_diff_size: Option<u64>,
    pub sierra_gas: Option<u64>,
    pub n_txs: Option<u64>,
    pub proving_gas: Option<u64>,
    pub receipt_l2_gas: Option<u64>,
}

impl ResourceDeltas {
    /// Returns true if any field has a positive delta.
    pub fn any_exceeded(&self) -> bool {
        self.l1_gas.is_some()
            || self.message_segment_length.is_some()
            || self.n_events.is_some()
            || self.state_diff_size.is_some()
            || self.sierra_gas.is_some()
            || self.n_txs.is_some()
            || self.proving_gas.is_some()
            || self.receipt_l2_gas.is_some()
    }
}

impl std::fmt::Display for ResourceDeltas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ResourceDeltas(l1_gas={:?}, msg_seg={:?}, n_events={:?}, state_diff={:?}, sierra_gas={:?}, n_txs={:?}, proving_gas={:?}, receipt_l2_gas={:?})",
            self.l1_gas,
            self.message_segment_length,
            self.n_events,
            self.state_diff_size,
            self.sierra_gas,
            self.n_txs,
            self.proving_gas,
            self.receipt_l2_gas,
        )
    }
}

/// Compare execution resources from ExecutionBox (er_x1) against re-execution (er_x2)
/// and block capacity limit (block_limit).
///
/// Priority order:
/// 1. If er_x1 > block_limit on any field → Fatal.
/// 2. Else if er_x1 > er_x2 on any field  → Warn.
/// 3. Otherwise                            → Ok.
pub fn compare_execution_resources(
    er_x1: &BouncerWeights,
    er_x2: &BouncerWeights,
    block_limit: &BouncerWeights,
) -> ExecutionResourceComparison {
    let limit_deltas = exceeds_limit(er_x1, block_limit);
    if limit_deltas.any_exceeded() {
        return ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { deltas: limit_deltas };
    }

    let reexec_deltas = exceeds_limit(er_x1, er_x2);
    if reexec_deltas.any_exceeded() {
        return ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { deltas: reexec_deltas };
    }

    ExecutionResourceComparison::Ok
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Returns per-field positive deltas where `lhs > rhs` (lhs - rhs), None otherwise.
fn exceeds_limit(lhs: &BouncerWeights, rhs: &BouncerWeights) -> ResourceDeltas {
    ResourceDeltas {
        l1_gas: usize_delta(lhs.l1_gas, rhs.l1_gas),
        message_segment_length: usize_delta(lhs.message_segment_length, rhs.message_segment_length),
        n_events: usize_delta(lhs.n_events, rhs.n_events),
        state_diff_size: usize_delta(lhs.state_diff_size, rhs.state_diff_size),
        sierra_gas: gas_amount_delta(lhs.sierra_gas, rhs.sierra_gas),
        n_txs: usize_delta(lhs.n_txs, rhs.n_txs),
        proving_gas: gas_amount_delta(lhs.proving_gas, rhs.proving_gas),
        receipt_l2_gas: gas_amount_delta(lhs.receipt_l2_gas, rhs.receipt_l2_gas),
    }
}

fn usize_delta(lhs: usize, rhs: usize) -> Option<u64> {
    if lhs > rhs {
        Some((lhs - rhs) as u64)
    } else {
        None
    }
}

fn gas_amount_delta(lhs: GasAmount, rhs: GasAmount) -> Option<u64> {
    if lhs.0 > rhs.0 {
        Some(lhs.0 - rhs.0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::bouncer::BouncerWeights;
    use starknet_api::execution_resources::GasAmount;

    fn weights(l1_gas: usize, sierra_gas: u64) -> BouncerWeights {
        BouncerWeights { l1_gas, sierra_gas: GasAmount(sierra_gas), ..BouncerWeights::empty() }
    }

    fn max_weights() -> BouncerWeights {
        BouncerWeights {
            l1_gas: 1_000,
            message_segment_length: 1_000,
            n_events: 1_000,
            state_diff_size: 1_000,
            sierra_gas: GasAmount(1_000_000),
            n_txs: 1_000,
            proving_gas: GasAmount(1_000_000),
            receipt_l2_gas: GasAmount(1_000_000),
        }
    }

    #[test]
    fn equal_resources_is_ok() {
        let w = weights(100, 500);
        assert_eq!(compare_execution_resources(&w, &w, &max_weights()), ExecutionResourceComparison::Ok);
    }

    #[test]
    fn er_x1_less_than_er_x2_is_ok() {
        let x1 = weights(50, 200);
        let x2 = weights(100, 500);
        assert_eq!(compare_execution_resources(&x1, &x2, &max_weights()), ExecutionResourceComparison::Ok);
    }

    #[test]
    fn er_x1_greater_than_er_x2_within_limit_is_warn() {
        let x1 = weights(200, 500);
        let x2 = weights(100, 500);
        let result = compare_execution_resources(&x1, &x2, &max_weights());
        assert!(matches!(result, ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { .. }));
        if let ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { deltas } = result {
            assert_eq!(deltas.l1_gas, Some(100));
            assert_eq!(deltas.sierra_gas, None);
        }
    }

    #[test]
    fn er_x1_exceeds_block_limit_is_fatal() {
        let limit = weights(100, 1_000);
        let x1 = weights(200, 500); // l1_gas exceeds limit
        let x2 = weights(50, 500);
        let result = compare_execution_resources(&x1, &x2, &limit);
        assert!(matches!(result, ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. }));
        if let ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { deltas } = result {
            assert_eq!(deltas.l1_gas, Some(100));
        }
    }

    #[test]
    fn fatal_takes_priority_over_warn() {
        // x1 > x2 (warn) AND x1 > limit (fatal) → fatal wins
        let limit = weights(100, 1_000);
        let x1 = weights(200, 500); // exceeds limit
        let x2 = weights(50, 500); // x1 also > x2
        let result = compare_execution_resources(&x1, &x2, &limit);
        assert!(matches!(result, ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. }));
    }

    #[test]
    fn receipt_l2_gas_above_limit_is_fatal() {
        let mut limit = max_weights();
        limit.receipt_l2_gas = GasAmount(100);
        let mut x1 = weights(10, 10);
        x1.receipt_l2_gas = GasAmount(101);
        let result = compare_execution_resources(&x1, &BouncerWeights::empty(), &limit);
        assert!(matches!(result, ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. }));
        if let ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { deltas } = result {
            assert_eq!(deltas.receipt_l2_gas, Some(1));
        }
    }
}
