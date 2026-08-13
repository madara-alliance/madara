use crate::CurrentBlockState;
use anyhow::Result;
use blockifier::bouncer::BouncerWeights;
use mc_db::close_pipeline_contract::CloseJobPayload as DbCloseJobPayload;
use mc_db::preconfirmed::PreconfirmedExecutedTransaction;
use mp_block::header::PreconfirmedHeader;
use std::time::Instant;
use tokio::sync::oneshot;

pub(crate) struct QueuedClosePayload {
    pub db_payload: DbCloseJobPayload,
    pub state: CurrentBlockState,
    /// Canonical bouncer weights selected by comparator (C-009B).
    /// In Mixed mode: EB weights on Accept/AcceptWithWarn, BRE weights on StopExecutionBox.
    /// In BlockifierOnly: always EB weights.
    pub canonical_bouncer_weights: BouncerWeights,
    pub state_diff: mp_state_update::StateDiff,
    /// C-024: Canonical executed rows for block X, selected by comparator.
    /// This is the single source of truth for the close-time block body.
    /// Close must use these rows directly instead of re-reading from preconfirmed
    /// sources (which may be stale under async runahead).
    pub canonical_executed_rows: Vec<PreconfirmedExecutedTransaction>,
    /// C-024: Header for block X at canonicalization time.
    pub canonical_header: PreconfirmedHeader,
    pub internal_capacity: usize,
    pub enqueued_at: Instant,
}

pub(crate) struct QueuedCloseJob {
    pub payload: QueuedClosePayload,
    pub completion: oneshot::Sender<Result<CloseJobCompletion>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CloseJobCompletion {
    pub block_n: u64,
}
