//! Executor thread internal logic.

use crate::fallback::types::{ExecutionMode, RuntimeReplayStatus};
use crate::metrics::BlockProductionMetrics;
use crate::util::{
    create_execution_context, BatchToExecute, BlockExecutionContext, ExecutionStats, RoutedBatchToExecute,
    TaintedRebuildCarryTx,
};
use anyhow::Context;
use blockifier::blockifier::transaction_executor::TransactionExecutor;
use futures::future::OptionFuture;
use mc_db::MadaraBackend;
use mc_exec::metrics::{context_label, metrics as exec_metrics, tx_type_to_label};
use mc_exec::{execution::TxInfo, LayeredStateAdapter};
use mc_rust_exec::{RustDeferredReason, RustExecRuntimeConfig, RustPhaseState};
use mp_convert::{Felt, ToFelt};
use opentelemetry::KeyValue;
use starknet_api::contract_class::ContractClass;
use starknet_api::core::ClassHash;
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
    time::Instant as StdInstant,
};
use tokio::{
    sync::{mpsc, mpsc::OwnedPermit, watch},
    time::Instant,
};

mod commands;
mod execute;
mod fallback;
mod new_block;
mod replies;
mod state;

struct ExecutorStateExecuting {
    exec_ctx: BlockExecutionContext,
    execution_mode: ExecutionMode,
    /// Note: We have a special StateAdaptor here. This is because saving the block to the database can actually lag a
    /// bit behind our execution. As such, any change that we make will need to be cached in our state adaptor so that
    /// we can be sure the state of the last block is always visible to the new one.
    executor: TransactionExecutor<LayeredStateAdapter>,
    declared_classes: HashMap<ClassHash, ContractClass>,
    consumed_l1_to_l2_nonces: HashSet<u64>,
    rust_phase_state: RustPhaseState,
    saw_blockifier_txs: bool,
    executed_in_block: BatchToExecute,
}

struct ExecutorStateNewBlock {
    /// Keep the cached adaptor around to keep the cache around.
    state_adaptor: LayeredStateAdapter,
    consumed_l1_to_l2_nonces: HashSet<u64>,
}

/// Note: The reason this exists is because we want to create the new block execution context (meaning, the block header) as late as possible, as to have
/// the best gas prices. This is especially important when the no_empty_block configuration is enabled, as otherwise we would end up:
/// - Creating a new execution context, using the current gas prices.
/// - Waiting for a transaction to arrive.... potentially for a very, very long time..
/// - Transaction arrives, we execute it and close the block, as the block_time is reached.
///
/// At that point, the gas prices would be all wrong! In order to support no_empty_block correctly, we have to delay execution context creation
/// until the first transaction has arrived.
#[allow(clippy::large_enum_variant)]
enum ExecutorThreadState {
    /// A block has been started.
    Executing(ExecutorStateExecuting),
    /// Intermediate state, we do not have initialized the execution yet.
    NewBlock(ExecutorStateNewBlock),
}

impl ExecutorThreadState {
    fn consumed_l1_to_l2_nonces(&mut self) -> &mut HashSet<u64> {
        match self {
            ExecutorThreadState::Executing(s) => &mut s.consumed_l1_to_l2_nonces,
            ExecutorThreadState::NewBlock(s) => &mut s.consumed_l1_to_l2_nonces,
        }
    }
    /// Returns a mutable reference to the state adapter.
    fn layered_state_adapter_mut(&mut self) -> &mut LayeredStateAdapter {
        match self {
            ExecutorThreadState::Executing(s) => {
                &mut s.executor.block_state.as_mut().expect("State already taken").state
            }
            ExecutorThreadState::NewBlock(s) => &mut s.state_adaptor,
        }
    }
}

/// Executor runs on a separate thread, as to avoid having tx popping, block closing etc. take precious time away that could
/// be spent executing the next tick instead.
/// This thread becomes the blockifier executor scheduler thread (via TransactionExecutor), which will internally spawn worker threads.
pub struct ExecutorThread {
    backend: Arc<MadaraBackend>,
    metrics: Arc<BlockProductionMetrics>,
    replay_mode_enabled: bool,

    incoming_batches: mpsc::Receiver<RoutedBatchToExecute>,
    replies_sender: mpsc::Sender<super::ExecutorMessage>,
    commands: mpsc::UnboundedReceiver<super::ExecutorCommand>,
    replay_status_tx: watch::Sender<RuntimeReplayStatus>,
    /// Watch sender/receiver for the effective execution mode used by the active block.
    /// The executor updates this at safe block boundaries.
    execution_mode_tx: watch::Sender<ExecutionMode>,
    execution_mode_rx: watch::Receiver<ExecutionMode>,
    execution_epoch_rx: watch::Receiver<u64>,
    start_tainted_rebuild_parked: bool,
    pipeline_mode: crate::BlockPipelineMode,
    rust_exec_runtime_config: RustExecRuntimeConfig,

    /// See `take_tx_batch`. When the mempool is empty, we will not be getting transactions.
    /// We still potentially want to emit empty blocks based on the block_time deadline.
    wait_rt: tokio::runtime::Runtime,
}

enum WaitTxBatchOutcome {
    /// Batch channel closed.
    Exit,
    /// Got a command to execute.
    Command(super::ExecutorCommand),
    /// Batch (routed; executor merges branches until T-031 two-phase execution is wired)
    Batch(RoutedBatchToExecute),
}

#[derive(Default)]
struct BatchBoundarySummary {
    first_hash: Option<String>,
    first_nonce: Option<String>,
    last_hash: Option<String>,
    last_nonce: Option<String>,
}

fn summarize_batch(batch: &BatchToExecute) -> BatchBoundarySummary {
    let mut summary = BatchBoundarySummary::default();
    if let Some(first) = batch.txs.first() {
        summary.first_hash = Some(format!("{:#x}", first.tx_hash().to_felt()));
        summary.first_nonce = Some(format!("{:#x}", first.nonce().to_felt()));
    }
    if let Some(last) = batch.txs.last() {
        summary.last_hash = Some(format!("{:#x}", last.tx_hash().to_felt()));
        summary.last_nonce = Some(format!("{:#x}", last.nonce().to_felt()));
    }
    summary
}

fn summarize_routed_batch(batch: &RoutedBatchToExecute) -> BatchBoundarySummary {
    let mut combined = BatchToExecute::default();
    combined.extend(batch.blockifier_batch.clone());
    combined.extend(batch.rust_batch.clone());
    summarize_batch(&combined)
}

fn summarize_carry_txs(carry: &[TaintedRebuildCarryTx]) -> BatchBoundarySummary {
    let mut combined = BatchToExecute::default();
    combined.extend(carry.iter().cloned().map(|carry_tx| (carry_tx.tx, carry_tx.additional_info)));
    summarize_batch(&combined)
}

fn extend_carry_txs(carry: &mut Vec<TaintedRebuildCarryTx>, batch: BatchToExecute, source_block_n: Option<u64>) {
    carry.extend(batch.into_iter().map(|(tx, additional_info)| TaintedRebuildCarryTx {
        tx,
        additional_info,
        source_block_n,
    }));
}

enum WaitForConfirmedOutcome {
    Advanced,
    Command(super::ExecutorCommand),
}

enum WaitForConfirmedHashOutcome {
    Hash(Option<(u64, Felt)>),
    ContinueOuterLoop,
}

enum NewBlockBoundarySyncOutcome {
    Proceed,
    ContinueOuterLoop,
}

enum ForwardReplySendOutcome {
    Sent,
    DroppedStale,
    ChannelClosed,
}

enum ForwardReplyReservation {
    Permit(OwnedPermit<super::ExecutorMessage>),
    EpochChanged(u64),
    EpochWatchClosed(u64),
    ChannelClosed,
}

impl ExecutorThread {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: Arc<MadaraBackend>,
        incoming_batches: mpsc::Receiver<super::RoutedBatchToExecute>,
        replies_sender: mpsc::Sender<super::ExecutorMessage>,
        commands: mpsc::UnboundedReceiver<super::ExecutorCommand>,
        metrics: Arc<BlockProductionMetrics>,
        replay_mode_enabled: bool,
        replay_status_tx: watch::Sender<RuntimeReplayStatus>,
        execution_mode_tx: watch::Sender<ExecutionMode>,
        execution_mode_rx: watch::Receiver<ExecutionMode>,
        execution_epoch_rx: watch::Receiver<u64>,
        start_tainted_rebuild_parked: bool,
        pipeline_mode: crate::BlockPipelineMode,
        rust_exec_runtime_config: RustExecRuntimeConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            backend,
            metrics,
            replay_mode_enabled,
            incoming_batches,
            replies_sender,
            commands,
            replay_status_tx,
            execution_mode_tx,
            execution_mode_rx,
            execution_epoch_rx,
            start_tainted_rebuild_parked,
            pipeline_mode,
            rust_exec_runtime_config,
            wait_rt: tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .context("Building tokio runtime")?,
        })
    }
}

#[cfg(test)]
mod tests;
