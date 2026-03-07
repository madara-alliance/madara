use crate::CurrentBlockState;
use anyhow::Result;
use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
use mc_db::close_pipeline_contract::CloseJobPayload as DbCloseJobPayload;
use mc_db::rocksdb::global_trie::in_memory::InMemoryRootComputation;
use std::time::Instant;
use tokio::sync::oneshot;

pub(crate) type TrieBatchComputeJoinHandle = tokio::task::JoinHandle<Result<Vec<InMemoryRootComputation>>>;

pub(crate) struct QueuedClosePayload {
    pub db_payload: DbCloseJobPayload,
    pub state: CurrentBlockState,
    pub block_exec_summary: Box<BlockExecutionSummary>,
    pub state_diff: mp_state_update::StateDiff,
    pub is_boundary: bool,
    pub trie_log_mode: mc_db::rocksdb::global_trie::in_memory::TrieLogMode,
    pub trie_batch_handle: Option<TrieBatchComputeJoinHandle>,
    pub trie_batch_index: Option<usize>,
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
