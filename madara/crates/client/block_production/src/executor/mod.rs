use crate::util::{BatchToExecute, BlockExecutionContext, ExecutionStats};
use anyhow::Context;
use blockifier::blockifier::transaction_executor::{
    BlockExecutionSummary, TransactionExecutionOutput, TransactionExecutorResult,
};
use mc_db::MadaraBackend;
use std::{any::Any, panic::AssertUnwindSafe, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver},
    oneshot,
};

mod tests;
pub(crate) mod thread;

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
    },
    BatchExecuted(BatchExecutionResult),
    EndBlock(BlockExecutionSummary),
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
    commands: UnboundedReceiver<ExecutorCommand>,
) -> anyhow::Result<ExecutorThreadHandle> {
    // buffer is 1.
    let (send_batch, incoming_batches) = mpsc::channel(1);
    let (replies_sender, replies_recv) = mpsc::channel(100);
    let (stop_sender, stop_recv) = oneshot::channel();

    let executor = thread::ExecutorThread::new(backend, incoming_batches, replies_sender, commands)?;
    std::thread::Builder::new()
        .name("executor".into())
        .spawn(move || stop_sender.send(std::panic::catch_unwind(AssertUnwindSafe(move || executor.run()))))
        .context("Error when spawning thread")?;

    Ok(ExecutorThreadHandle { send_batch: Some(send_batch), replies: replies_recv, stop: StopErrorReceiver(stop_recv) })
}
