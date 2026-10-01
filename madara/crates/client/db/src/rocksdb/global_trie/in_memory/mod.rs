//! Root computation over a pinned checkpoint plus a private mutable Bonsai overlay.
//!
//! A root job applies the cumulative state diff since its selected checkpoint. It never changes
//! the live database. The resulting overlay is a delta against that checkpoint, not a complete
//! database snapshot and not a delta against the immediately preceding root job.
//!
//! Database clones within one job share overlay ownership; independent jobs require separate
//! overlays. Only the ordered finalizer may flush a completed overlay. Boundary flushes include
//! all three tries and durable checkpoint bookkeeping before confirmation becomes visible.

mod compute;
mod db;
mod overlay;
mod state_diff;
#[cfg(test)]
mod tests;

pub use compute::{
    compute_root_from_snapshot, compute_root_from_snapshot_sequential, compute_roots_in_parallel_from_snapshot,
    InMemoryRootComputation,
};
pub use db::{InMemoryBonsaiDb, InMemoryColumnMapping, OverlayKey, OverlayMap};
pub use overlay::{flush_overlay_and_checkpoint, BonsaiOverlay, BoundaryFlushOutcome};
pub use state_diff::{cumulative_squashed_state_diffs, squash_state_diffs};
