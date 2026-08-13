use crate::fallback::manager::{EnableError, EnableOutcome};
use crate::fallback::types::{ExecutionMode, ExecutionboxStatus, RuntimeReplayStatus};
use crate::metrics::BlockProductionMetrics;
use crate::util::{BatchToExecute, BlockExecutionContext, ExecutionStats, RoutedBatchToExecute, TaintedRebuildCarryTx};
use anyhow::Context;
use blockifier::blockifier::transaction_executor::{
    BlockExecutionSummary, TransactionExecutionOutput, TransactionExecutorResult,
};
use mc_db::MadaraBackend;
use mc_rust_exec::RustExecRuntimeConfig;
use std::{any::Any, panic::AssertUnwindSafe, sync::Arc, time::Instant};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver},
    oneshot, watch,
};

mod tests;
pub(crate) mod thread;

/// Handle to used to talk with the executor thread.
pub struct ExecutorThreadHandle {
    /// Input transactions need to be sent to this sender channel.
    /// Closing this channel will tell the executor thread to stop.
    pub send_batch: Option<mpsc::Sender<RoutedBatchToExecute>>,
    /// Receive the resulting Result of the thread.
    pub stop: StopErrorReceiver,
    /// Channel with the replies from the executor thread.
    pub replies: mpsc::Receiver<ExecutorMessage>,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ExecutorCommandError {
    #[error("Executor not running")]
    ChannelClosed,
    #[error("Executor is parked for tainted rebuild recovery")]
    TaintedRebuildActive,
    #[error("Invalid tainted rebuild resume: {0}")]
    InvalidTaintedRebuildResume(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaintedRebuildResumeAck {
    pub confirmed_head: u64,
    pub next_block_n: u64,
    pub execution_epoch: u64,
}

#[derive(Debug)]
pub enum ExecutorCommand {
    /// Force close the current block.
    CloseBlock(oneshot::Sender<Result<(), ExecutorCommandError>>),
    /// Update the desired execution mode for future blocks.
    /// The executor applies it immediately only when no block is active.
    SetDesiredExecutionMode { mode: ExecutionMode },
    /// Fail-safe fallback entered for block `block_n`; discard any newer speculative
    /// forward block, drain executor-local pending work, and return the one-time carry
    /// handoff to block production under the provided execution epoch.
    PrepareTaintedRebuildFallback {
        block_n: u64,
        execution_epoch: u64,
        reply: oneshot::Sender<Result<Vec<TaintedRebuildCarryTx>, ExecutorCommandError>>,
    },
    /// The durable tainted rebuild is fully confirmed. Re-anchor to the backend,
    /// acknowledge the resulting frontier, and leave the parked state.
    ResumeAfterTaintedRebuild {
        expected_confirmed_head: u64,
        execution_epoch: u64,
        reply: oneshot::Sender<Result<TaintedRebuildResumeAck, ExecutorCommandError>>,
    },
}

/// Commands sent from BlockProductionHandle to BlockProductionTask's main loop
/// for ExecutionBox mode control. Each variant carries a response channel.
#[derive(Debug)]
pub enum FallbackCommand {
    /// Enable ExecutionBox (synchronous decision: replay_in_progress, already_mixed, or enabled_now).
    Enable(oneshot::Sender<Result<EnableOutcome, EnableError>>),
    /// Force disable ExecutionBox (idempotent).
    Disable(oneshot::Sender<()>),
    /// Query current ExecutionBox status snapshot.
    Status(oneshot::Sender<ExecutionboxStatus>),
}

#[derive(Debug)]
/// Actor model messages, sent between the block production and itself to drive the production of
/// new blocks.
///
/// We use this since the block production is parallelized and message passing allows for easy
/// communication between the execution thread and the master thread.
pub enum ExecutorMessage {
    /// Asks the block production task to start a new block.
    StartNewBlock {
        /// The proto-header. It's exactly like PreconfirmedHeader, but it does not have the parent_block_hash field because it's not known yet.
        exec_ctx: BlockExecutionContext,
        /// Frozen execution mode snapshot for this block.
        execution_mode: ExecutionMode,
        /// Execution epoch used to discard stale forward messages after fallback.
        execution_epoch: u64,
    },
    BatchExecuted(BatchExecutionResult),
    /// Normal block closing (block time reached, block full, or explicit CloseBlock).
    EndBlock {
        block_exec_summary: Box<BlockExecutionSummary>,
        block_number: u64,
        execution_epoch: u64,
    },
    /// Final block closing during graceful shutdown. Only sent when executor detects shutdown.
    /// - Some(summary): Block exists and was finalized, close it
    /// - None: No block exists, executor is just signaling completion
    EndFinalBlock {
        block_exec_summary: Option<Box<BlockExecutionSummary>>,
        block_number: Option<u64>,
        execution_epoch: u64,
    },
}

#[derive(Debug)]
pub struct BatchExecutionResult {
    pub executed_txs: BatchToExecute,
    pub original_tx_hashes: Vec<mp_convert::Felt>,
    pub blockifier_results: Vec<TransactionExecutorResult<TransactionExecutionOutput>>,
    pub stats: ExecutionStats,
    pub execution_mode: ExecutionMode,
    pub execution_epoch: u64,
    pub emitted_at: Instant,
}

/// Receiver for the stop condition of the executor thread.
pub struct StopErrorReceiver(oneshot::Receiver<Result<anyhow::Result<()>, Box<dyn Any + Send + 'static>>>);
impl StopErrorReceiver {
    pub async fn recv(&mut self) -> anyhow::Result<()> {
        match (&mut self.0).await {
            Ok(Ok(res)) => res,
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            Err(_) => Ok(()), // channel closed
        }
    }
}
/// Create the executor thread and returns a handle to it.
#[allow(clippy::too_many_arguments)]
pub fn start_executor_thread(
    backend: Arc<MadaraBackend>,
    commands: UnboundedReceiver<ExecutorCommand>,
    metrics: Arc<BlockProductionMetrics>,
    replay_mode_enabled: bool,
    replay_status_tx: watch::Sender<RuntimeReplayStatus>,
    execution_mode_tx: watch::Sender<ExecutionMode>,
    execution_mode_rx: watch::Receiver<ExecutionMode>,
    execution_epoch_rx: watch::Receiver<u64>,
    start_tainted_rebuild_parked: bool,
    rust_exec_runtime_config: RustExecRuntimeConfig,
) -> anyhow::Result<ExecutorThreadHandle> {
    // buffer is 1.
    let (send_batch, incoming_batches) = mpsc::channel::<RoutedBatchToExecute>(1);
    let (replies_sender, replies_recv) = mpsc::channel(100);
    let (stop_sender, stop_recv) = oneshot::channel();

    let executor = thread::ExecutorThread::new(
        backend,
        incoming_batches,
        replies_sender,
        commands,
        metrics,
        replay_mode_enabled,
        replay_status_tx,
        execution_mode_tx,
        execution_mode_rx,
        execution_epoch_rx,
        start_tainted_rebuild_parked,
        rust_exec_runtime_config,
    )?;
    // TODO(heemankv, 28-10-25): We should not use std thread builder over a tokio mpsc context, might not be stable
    std::thread::Builder::new()
        .name("executor".into())
        .spawn(move || stop_sender.send(std::panic::catch_unwind(AssertUnwindSafe(move || executor.run()))))
        .context("Error when spawning thread")?;

    Ok(ExecutorThreadHandle { send_batch: Some(send_batch), replies: replies_recv, stop: StopErrorReceiver(stop_recv) })
}
