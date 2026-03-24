use crate::CurrentBlockState;
use anyhow::Result;
use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
use mc_db::close_pipeline_contract::CloseJobPayload as DbCloseJobPayload;
use mc_db::rocksdb::SnapshotRef;
use mp_state_update::StateDiff;
use std::time::Instant;
use tokio::sync::oneshot;

pub(crate) struct QueuedClosePayload {
    pub db_payload: DbCloseJobPayload,
    pub state: CurrentBlockState,
    pub block_exec_summary: Box<BlockExecutionSummary>,
    pub state_diff: StateDiff,
    pub is_boundary: bool,
    pub trie_log_mode: mc_db::rocksdb::global_trie::in_memory::TrieLogMode,
    pub compare_parallel_with_sequential: bool,
    pub root_base_block_n: Option<u64>,
    pub root_snapshot: Option<SnapshotRef>,
    pub root_state_diffs: Vec<StateDiff>,
    pub last_execution_finished_at: Option<Instant>,
    pub close_block_received_at: Instant,
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
