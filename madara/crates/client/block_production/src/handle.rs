use crate::executor::{self, ExecutorCommand, ExecutorCommandError, FallbackCommand};
use crate::fallback::manager::{EnableError, EnableOutcome};
use crate::fallback::types::ExecutionMode;
use crate::fallback::types::ExecutionboxStatus;
use crate::util::TaintedRebuildCarryTx;
use crate::MempoolIntakeMode;
use async_trait::async_trait;
use mc_db::MadaraBackend;
use mc_submit_tx::{
    SubmitL1HandlerTransaction, SubmitTransaction, SubmitTransactionError, SubmitValidatedTransaction,
    TransactionLookup, TransactionValidator, TransactionValidatorConfig,
};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_10_2::BroadcastedInvokeTxn;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::{L1HandlerTransactionResult, L1HandlerTransactionWithFee};
use std::{sync::Arc, time::Instant};
use tokio::sync::watch;
use tokio::sync::{mpsc, oneshot};

const BYPASS_ENQUEUE_WARN_MS: f64 = 25.0;

struct BypassInput(mpsc::Sender<ValidatedTransaction>);

#[async_trait]
impl SubmitValidatedTransaction for BypassInput {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError> {
        let tx_hash = tx.hash;
        let available_capacity_before_send = self.0.capacity();
        let send_started = Instant::now();
        self.0.send(tx).await.map_err(|e| SubmitTransactionError::Internal(anyhow::anyhow!(e)))?;
        let send_wait_ms = send_started.elapsed().as_secs_f64() * 1000.0;
        if send_wait_ms >= BYPASS_ENQUEUE_WARN_MS {
            tracing::warn!(
                "bypass_input_tx_slow_enqueue tx_hash={tx_hash:#x} available_capacity_before_send={} available_capacity_after_send={} send_wait_ms={send_wait_ms}",
                available_capacity_before_send,
                self.0.capacity()
            );
        } else {
            tracing::debug!(
                "bypass_input_tx_enqueued tx_hash={tx_hash:#x} available_capacity_before_send={} available_capacity_after_send={} send_wait_ms={send_wait_ms}",
                available_capacity_before_send,
                self.0.capacity()
            );
        }
        Ok(())
    }
}

#[async_trait]
impl TransactionLookup for BypassInput {
    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        None
    }

    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        None
    }
}

#[derive(Clone, Debug)]
/// Remotely control block production.
pub struct BlockProductionHandle {
    /// Commands to executor task.
    executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
    /// Commands to BlockProductionTask main loop for ExecutionBox mode control.
    fallback_commands: mpsc::UnboundedSender<FallbackCommand>,
    bypass_input: mpsc::Sender<ValidatedTransaction>,
    mempool_intake_tx: watch::Sender<MempoolIntakeMode>,
    /// We use TransactionValidator to handle conversion to blockifier, class compilation etc. Mostly for convenience.
    tx_converter: Arc<TransactionValidator>,
}

impl BlockProductionHandle {
    pub(crate) fn new(
        backend: Arc<MadaraBackend>,
        executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
        fallback_commands: mpsc::UnboundedSender<FallbackCommand>,
        bypass_input: mpsc::Sender<ValidatedTransaction>,
        mempool_intake_tx: watch::Sender<MempoolIntakeMode>,
        no_charge_fee: bool,
    ) -> Self {
        Self {
            executor_commands,
            fallback_commands,
            bypass_input: bypass_input.clone(),
            mempool_intake_tx,
            tx_converter: TransactionValidator::new(
                Arc::new(BypassInput(bypass_input)),
                backend,
                TransactionValidatorConfig { disable_validation: true, disable_fee: no_charge_fee },
            )
            .into(),
        }
    }

    /// Force the current block to close without waiting for block time.
    pub async fn close_block(&self) -> Result<(), ExecutorCommandError> {
        let (sender, recv) = oneshot::channel();
        self.executor_commands
            .send(ExecutorCommand::CloseBlock(sender))
            .map_err(|_| ExecutorCommandError::ChannelClosed)?;
        recv.await.map_err(|_| ExecutorCommandError::ChannelClosed)?
    }

    /// Returns the executor-local carry that must be merged into the durable tainted rebuild handoff.
    pub(crate) fn request_tainted_rebuild_fallback(
        &self,
        block_n: u64,
        execution_epoch: u64,
    ) -> Result<oneshot::Receiver<Result<Vec<TaintedRebuildCarryTx>, ExecutorCommandError>>, ExecutorCommandError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.executor_commands
            .send(ExecutorCommand::PrepareTaintedRebuildFallback { block_n, execution_epoch, reply: reply_tx })
            .map_err(|_| ExecutorCommandError::ChannelClosed)?;
        Ok(reply_rx)
    }

    pub(crate) fn set_desired_execution_mode(&self, mode: ExecutionMode) -> Result<(), ExecutorCommandError> {
        self.executor_commands
            .send(ExecutorCommand::SetDesiredExecutionMode { mode })
            .map_err(|_| ExecutorCommandError::ChannelClosed)
    }

    pub(crate) fn resume_after_tainted_rebuild(
        &self,
        expected_confirmed_head: u64,
        execution_epoch: u64,
    ) -> Result<
        oneshot::Receiver<Result<crate::executor::TaintedRebuildResumeAck, ExecutorCommandError>>,
        ExecutorCommandError,
    > {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.executor_commands
            .send(ExecutorCommand::ResumeAfterTaintedRebuild {
                expected_confirmed_head,
                execution_epoch,
                reply: reply_tx,
            })
            .map_err(|_| ExecutorCommandError::ChannelClosed)?;
        Ok(reply_rx)
    }

    pub fn set_mempool_intake(&self, enabled: bool) -> anyhow::Result<()> {
        let mode = if enabled { MempoolIntakeMode::Running } else { MempoolIntakeMode::Paused };
        let previous_mode = *self.mempool_intake_tx.borrow();
        self.mempool_intake_tx.send(mode).map_err(|e| anyhow::anyhow!("Mempool intake channel closed: {e}"))?;
        if previous_mode != mode {
            tracing::info!(previous_mode = ?previous_mode, new_mode = ?mode, "mempool_intake_updated");
        } else {
            tracing::debug!(mode = ?mode, "mempool_intake_already_set");
        }
        Ok(())
    }

    pub fn flush_mempool(&self) -> anyhow::Result<()> {
        self.mempool_intake_tx
            .send(MempoolIntakeMode::FlushOnce)
            .map_err(|e| anyhow::anyhow!("Mempool intake channel closed: {e}"))?;
        Ok(())
    }

    pub(crate) fn mempool_intake_tx(&self) -> watch::Sender<MempoolIntakeMode> {
        self.mempool_intake_tx.clone()
    }

    /// Send a transaction through the bypass channel to bypass mempool and validation.
    pub async fn send_tx_raw(&self, tx: ValidatedTransaction) -> Result<(), ExecutorCommandError> {
        self.bypass_input.send(tx).await.map_err(|_| ExecutorCommandError::ChannelClosed)
    }

    /// Enable ExecutionBox (synchronous decision per design).
    ///
    /// Returns `Ok(EnableOutcome)` on success (EnabledNow or AlreadyMixed),
    /// or `Err(EnableError::ReplayInProgress)` when startup recovery or replay backlog is active.
    /// Outer `Err(ExecutorCommandError)` signals that the block production task is not running.
    pub async fn executionbox_enable(&self) -> Result<Result<EnableOutcome, EnableError>, ExecutorCommandError> {
        let (tx, rx) = oneshot::channel();
        self.fallback_commands.send(FallbackCommand::Enable(tx)).map_err(|_| ExecutorCommandError::ChannelClosed)?;
        rx.await.map_err(|_| ExecutorCommandError::ChannelClosed)
    }

    /// Force disable ExecutionBox. Idempotent if already disabled.
    pub async fn executionbox_disable(&self) -> Result<(), ExecutorCommandError> {
        let (tx, rx) = oneshot::channel();
        self.fallback_commands.send(FallbackCommand::Disable(tx)).map_err(|_| ExecutorCommandError::ChannelClosed)?;
        rx.await.map_err(|_| ExecutorCommandError::ChannelClosed)
    }

    /// Query current ExecutionBox status snapshot.
    pub async fn executionbox_status(&self) -> Result<ExecutionboxStatus, ExecutorCommandError> {
        let (tx, rx) = oneshot::channel();
        self.fallback_commands.send(FallbackCommand::Status(tx)).map_err(|_| ExecutorCommandError::ChannelClosed)?;
        rx.await.map_err(|_| ExecutorCommandError::ChannelClosed)
    }
}

// For convenience, we proxy the submit tx traits.

#[async_trait]
impl SubmitTransaction for BlockProductionHandle {
    async fn submit_declare_v0_transaction(
        &self,
        tx: BroadcastedDeclareTxnV0,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        self.tx_converter.submit_declare_v0_transaction(tx).await
    }
    async fn submit_declare_transaction(
        &self,
        tx: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        self.tx_converter.submit_declare_transaction(tx).await
    }
    async fn submit_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        self.tx_converter.submit_deploy_account_transaction(tx).await
    }
    async fn submit_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        self.tx_converter.submit_invoke_transaction(tx).await
    }
}

#[async_trait]
impl SubmitL1HandlerTransaction for BlockProductionHandle {
    async fn submit_l1_handler_transaction(
        &self,
        tx: L1HandlerTransactionWithFee,
    ) -> Result<L1HandlerTransactionResult, SubmitTransactionError> {
        self.tx_converter.submit_l1_handler_transaction(tx).await
    }
}

#[async_trait]
impl SubmitValidatedTransaction for BlockProductionHandle {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError> {
        self.send_tx_raw(tx).await.map_err(|e| SubmitTransactionError::Internal(anyhow::anyhow!(e)))
    }
}

#[async_trait]
impl TransactionLookup for BlockProductionHandle {
    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        None
    }

    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_chain_config::ChainConfig;

    fn make_handle() -> (BlockProductionHandle, watch::Receiver<MempoolIntakeMode>) {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let (executor_tx, _executor_rx) = mpsc::unbounded_channel();
        let (fallback_tx, _fallback_rx) = mpsc::unbounded_channel::<FallbackCommand>();
        let (bypass_tx, _bypass_rx) = mpsc::channel(4);
        let (mempool_intake_tx, mempool_intake_rx) = watch::channel(MempoolIntakeMode::Paused);

        (
            BlockProductionHandle::new(backend, executor_tx, fallback_tx, bypass_tx, mempool_intake_tx, false),
            mempool_intake_rx,
        )
    }

    #[test]
    fn flush_mempool_switches_intake_mode_to_flush_once() {
        let (handle, mempool_intake_rx) = make_handle();

        handle.flush_mempool().expect("flush should succeed");

        assert_eq!(*mempool_intake_rx.borrow(), MempoolIntakeMode::FlushOnce);
    }

    #[test]
    fn set_mempool_intake_false_switches_to_paused() {
        let (handle, mempool_intake_rx) = make_handle();

        handle.set_mempool_intake(false).expect("set paused should succeed");

        assert_eq!(*mempool_intake_rx.borrow(), MempoolIntakeMode::Paused);
    }

    #[test]
    fn set_mempool_intake_true_switches_to_running() {
        let (handle, mempool_intake_rx) = make_handle();

        handle.set_mempool_intake(true).expect("set running should succeed");

        assert_eq!(*mempool_intake_rx.borrow(), MempoolIntakeMode::Running);
    }
}
