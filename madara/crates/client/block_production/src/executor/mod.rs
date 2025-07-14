use crate::util::{BatchToExecute, BlockExecutionContext, ExecutionStats};
use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::{TransactionExecutionOutput, TransactionExecutorResult},
    state::cached_state::StorageEntry,
};
use mc_db::MadaraBackend;
use mc_mempool::L1DataProvider;
use mp_convert::Felt;
use std::{any::Any, collections::HashMap, panic::AssertUnwindSafe, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver},
    oneshot,
};

mod tests;
mod thread;

/// Handle to used to talk with the executor thread.
pub struct ExecutorThreadHandle {
    /// Input transactions need to be sent to this sender channel.
    /// Closing this channel will tell the executor thread to stop.
    pub send_batch: Option<mpsc::Sender<BatchToExecute>>,
    /// Receive the resulting Result of the thread.
    pub stop: StopErrorReceiver,
    /// Channel with the replies from the executor thread.
    pub replies: mpsc::Receiver<ExecutorMessage>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutorCommandError {
    #[error("Executor not running")]
    ChannelClosed,
}

#[derive(Debug)]
pub enum ExecutorCommand {
    /// Force close the current block.
    CloseBlock(oneshot::Sender<Result<(), ExecutorCommandError>>),
}

#[derive(Debug)]
/// Executor thread => master task
pub enum ExecutorMessage {
    StartNewBlock {
        /// Used to add the block_n-10 block hash table entry to the state diff.
        initial_state_diffs_storage: HashMap<StorageEntry, Felt>,
        /// The proto-header. It's exactly like PendingHeader, but it does not have the parent_block_hash field because it's not known yet.
        exec_ctx: BlockExecutionContext,
    },
    BatchExecuted(BatchExecutionResult),
    EndBlock,
}

#[derive(Default, Debug)]
pub struct BatchExecutionResult {
    pub executed_txs: BatchToExecute,
    pub blockifier_results: Vec<TransactionExecutorResult<TransactionExecutionOutput>>,
    pub stats: ExecutionStats,
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
pub fn start_executor_thread(
    backend: Arc<MadaraBackend>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    commands: UnboundedReceiver<ExecutorCommand>,
) -> anyhow::Result<ExecutorThreadHandle> {
    // buffer is 1.
    let (send_batch, incoming_batches) = mpsc::channel(1);
    let (replies_sender, replies_recv) = mpsc::channel(100);
    let (stop_sender, stop_recv) = oneshot::channel();

    let executor = thread::ExecutorThread::new(backend, l1_data_provider, incoming_batches, replies_sender, commands)?;
    std::thread::Builder::new()
        .name("executor".into())
        .spawn(move || stop_sender.send(std::panic::catch_unwind(AssertUnwindSafe(move || executor.run()))))
        .context("Error when spawning thread")?;

    Ok(ExecutorThreadHandle { send_batch: Some(send_batch), replies: replies_recv, stop: StopErrorReceiver(stop_recv) })
}
