//! Close-pipeline orchestration and ordered database commit stages.
//!
//! Execution may run ahead and parallel Merkle roots may be prepared out of
//! order, but the confirmed head advances only after ordered commit succeeds.

mod metrics;
mod parallel;
mod reply;
mod serial;

pub(super) use parallel::{
    prune_diffs_since_snapshot, validate_parallel_queue_invariant, ParallelComputedClosePayload,
};

#[cfg(test)]
pub(super) use parallel::{collect_diffs_for_root_from_base, parallel_computed_payload_for_test};
