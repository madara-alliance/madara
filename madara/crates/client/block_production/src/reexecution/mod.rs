/// T-053: Async re-execution dispatcher and blocking worker.
///
/// Architecture (from design doc §Runtime Topology):
/// - `ReexecDispatcher` (Tokio orchestration): receives requests, enforces bounded in-flight
///   window, submits compute jobs via `spawn_blocking`.
/// - `run_blockifier_reexec` (blocking pool): reconstructs block context, runs Blockifier-only
///   execution in chunks with cooperative cancellation.
/// - `CancellationToken` per epoch: any fatal source (comparator stop, worker failure, shutdown)
///   cancels all in-flight re-exec workers for that epoch.
pub mod dispatcher;
#[cfg(test)]
mod tests;
pub mod worker;

use blockifier::bouncer::BouncerWeights;
use mc_db::MadaraBackend;
use mc_exec::ReexecParentOverlay;
use mp_convert::Felt;
use mp_receipt::TransactionReceipt;
use mp_state_update::{StateDiff, TransactionStateUpdate};
use std::collections::HashSet;
use std::sync::Arc;

use crate::util::BlockExecutionContext;

/// Maximum number of in-flight re-execution jobs (Starknet block_hash window = 10 blocks).
pub const MAX_INFLIGHT_REEXEC_BLOCKS: usize = 9;

/// Chunk size for iterating over transactions in the blocking worker.
/// Cancellation is checked between chunks.
pub const REEXEC_CHUNK_SIZE: usize = 50;

/// A re-execution request for a single block.
///
/// Carries everything the blocking worker needs to reconstruct block X from scratch
/// using only Blockifier, without touching any ExecutionBox state.
#[derive(Debug)]
pub struct ReexecRequest {
    /// Epoch for stale-result detection: results from a cancelled/old epoch are discarded.
    pub epoch: u64,
    /// The block number being re-executed.
    pub block_n: u64,
    /// Backend for creating state adapter (reads the parent/pre-block-X confirmed state).
    pub backend: Arc<MadaraBackend>,
    /// Exact block header context preserved from the original execution.
    pub exec_ctx: BlockExecutionContext,
    /// Pre-converted blockifier transactions in original tx order.
    pub txs: Vec<blockifier::transaction::transaction_execution::Transaction>,
    /// Set of contract addresses that were newly deployed in this block.
    /// Required for correct StateDiff construction (deployed vs replaced classification).
    pub deployed_contracts_set: HashSet<Felt>,
    /// Old declared contracts (pre-v0.13.2 format) in this block.
    pub old_declared_contracts: Vec<Felt>,
    /// Confirmed base block number for synthetic parent state construction.
    /// None means pre-genesis (no confirmed blocks yet).
    pub confirmed_base_block_n: Option<u64>,
    /// Ordered overlay diffs for accepted-but-unconfirmed blocks between confirmed
    /// base and target block. Required for correct re-execution under runahead
    /// runahead where confirmed tip lags behind executed blocks.
    pub parent_overlays: Vec<ReexecParentOverlay>,
}

/// Per-tx execution artifacts produced by BRE (C-013).
///
/// Each entry corresponds to one successfully executed transaction in block order.
/// Combined with original tx metadata (payload, arrived_at, paid_fee_on_l1, declared_class)
/// to produce BRE-backed `PreconfirmedExecutedTransaction` rows on stop path.
#[derive(Debug)]
pub struct ReexecExecutedTxArtifacts {
    /// Receipt converted from Blockifier `TransactionExecutionInfo`.
    pub receipt: TransactionReceipt,
    /// Per-tx state update from Blockifier `StateMaps`.
    /// Note: `declared_classes` is left empty here; the caller merges it from original metadata.
    pub tx_state_update: TransactionStateUpdate,
}

/// The output of a successful re-execution.
#[derive(Debug)]
pub struct ReexecResult {
    pub epoch: u64,
    /// State diff produced by Blockifier-only re-execution (SD-x2).
    pub state_diff: StateDiff,
    /// Execution resources (bouncer weights) produced by Blockifier-only re-execution (ER-x2).
    pub exec_resources: BouncerWeights,
    /// Ordered per-tx execution artifacts (C-013).
    ///
    /// This is the canonical included prefix produced by Blockifier re-execution.
    /// If Blockifier truncates the speculative block due to bouncer/capacity, `per_tx.len()`
    /// is the included prefix length and the speculative suffix must be replayed in the next block.
    ///
    /// Empty means no canonical per-tx artifacts were usable.
    pub per_tx: Vec<ReexecExecutedTxArtifacts>,
}

/// The outcome of a single re-execution worker job.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ReexecWorkerOutcome {
    /// Re-execution completed successfully.
    Completed(ReexecResult),
    /// Re-execution was canceled before producing a result.
    Cancelled { epoch: u64, block_n: u64 },
}
