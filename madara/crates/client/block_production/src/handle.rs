use crate::executor::{self, BatchExecutionOutcome, ExecutorCommand, ExecutorCommandError};
use crate::util::{AdditionalTxInfo, BatchToExecute};
use async_trait::async_trait;
use mc_db::MadaraBackend;
use mc_submit_tx::{
    SubmitL1HandlerTransaction, SubmitTransaction, SubmitTransactionError, SubmitValidatedTransaction,
    TransactionValidator, TransactionValidatorConfig,
};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_10_2::{BroadcastedInvokeTxn, BroadcastedTxn};
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::{L1HandlerTransactionResult, L1HandlerTransactionWithFee};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

struct BypassInput(mpsc::Sender<ValidatedTransaction>);

#[async_trait]
impl SubmitValidatedTransaction for BypassInput {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError> {
        self.0.send(tx).await.map_err(|e| SubmitTransactionError::Internal(anyhow::anyhow!(e)))
    }
    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        None
    }
    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        None
    }
}

/// Captures the [`ValidatedTransaction`]s produced by a [`TransactionValidator`] instead of
/// forwarding them to the mempool, so we can collect a whole batch and hand it to the executor as a
/// single contiguous unit. Used only by [`BlockProductionHandle::submit_transaction_batch`].
#[derive(Default)]
struct BatchCollector(Mutex<Vec<ValidatedTransaction>>);

#[async_trait]
impl SubmitValidatedTransaction for BatchCollector {
    async fn submit_validated_transaction(&self, tx: ValidatedTransaction) -> Result<(), SubmitTransactionError> {
        self.0.lock().expect("BatchCollector lock poisoned").push(tx);
        Ok(())
    }
    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        None
    }
    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        None
    }
}

/// Error returned when submitting a transaction batch for contiguous execution.
#[derive(Debug, thiserror::Error)]
pub enum BatchSubmitError {
    #[error("Transaction batch is empty")]
    EmptyBatch,
    #[error("Transaction batch too large: {len} transactions (max {max})")]
    BatchTooLarge { len: usize, max: usize },
    #[error("Transaction at index {index} failed validation: {reason}")]
    Validation { index: usize, reason: String },
    #[error("Internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Clone, Debug)]
/// Remotely control block production.
pub struct BlockProductionHandle {
    /// Commands to executor task.
    executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
    bypass_input: mpsc::Sender<ValidatedTransaction>,
    /// We use TransactionValidator to handle conversion to blockifier, class compilation etc. Mostly for convenience.
    tx_converter: Arc<TransactionValidator>,
    /// Backend, used to build a validation-enabled converter and read config for batch submission.
    backend: Arc<MadaraBackend>,
    /// Whether fees are disabled for this node (mirrored into the batch validator config).
    no_charge_fee: bool,
    /// Serializes transaction-batch submissions: only one batch executes at a time.
    batch_lock: Arc<tokio::sync::Mutex<()>>,
}

impl BlockProductionHandle {
    pub(crate) fn new(
        backend: Arc<MadaraBackend>,
        executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
        bypass_input: mpsc::Sender<ValidatedTransaction>,
        no_charge_fee: bool,
    ) -> Self {
        Self {
            executor_commands,
            bypass_input: bypass_input.clone(),
            tx_converter: TransactionValidator::new(
                Arc::new(BypassInput(bypass_input)),
                backend.clone(),
                TransactionValidatorConfig { disable_validation: true, disable_fee: no_charge_fee },
            )
            .into(),
            backend,
            no_charge_fee,
            batch_lock: Arc::new(tokio::sync::Mutex::new(())),
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

    /// Send a transaction through the bypass channel to bypass mempool and validation.
    pub async fn send_tx_raw(&self, tx: ValidatedTransaction) -> Result<(), ExecutorCommandError> {
        self.bypass_input.send(tx).await.map_err(|_| ExecutorCommandError::ChannelClosed)
    }

    /// Submit a batch of transactions to be executed contiguously: all the transactions are fed to
    /// the executor back-to-back, in submission order, with no foreign transaction executed between
    /// any two of them (across block boundaries if the batch does not fit in a single block).
    ///
    /// This is an *ordering* guarantee, not an atomicity one: individual transactions may still
    /// revert or be rejected during execution. The returned outcomes report, in submission order,
    /// what happened to each transaction.
    ///
    /// Each transaction is fully validated (signature/nonce/fee) before execution; if any fails
    /// validation, the whole submission is rejected (a dependent sequence with a hole is
    /// meaningless). Submissions are serialized: only one batch executes at a time.
    pub async fn submit_transaction_batch(
        &self,
        txs: Vec<BroadcastedTxn>,
    ) -> Result<BatchExecutionOutcome, BatchSubmitError> {
        if txs.is_empty() {
            return Err(BatchSubmitError::EmptyBatch);
        }
        let max = self.backend.chain_config().max_transaction_batch_size;
        if txs.len() > max {
            return Err(BatchSubmitError::BatchTooLarge { len: txs.len(), max });
        }

        // Validate + convert each transaction in order, capturing the resulting
        // `ValidatedTransaction`s instead of forwarding them to the mempool.
        let collector = Arc::new(BatchCollector::default());
        let validator = TransactionValidator::new(
            collector.clone(),
            self.backend.clone(),
            TransactionValidatorConfig { disable_validation: false, disable_fee: self.no_charge_fee },
        );
        for (index, tx) in txs.into_iter().enumerate() {
            let res = match tx {
                BroadcastedTxn::Invoke(tx) => validator.submit_invoke_transaction(tx).await.map(|_| ()),
                BroadcastedTxn::Declare(tx) => validator.submit_declare_transaction(tx).await.map(|_| ()),
                BroadcastedTxn::DeployAccount(tx) => validator.submit_deploy_account_transaction(tx).await.map(|_| ()),
            };
            res.map_err(|err| BatchSubmitError::Validation { index, reason: err.to_string() })?;
        }
        let validated = std::mem::take(&mut *collector.0.lock().expect("BatchCollector lock poisoned"));

        // Convert the validated transactions into a single contiguous execution batch.
        let mut batch = BatchToExecute::with_capacity(validated.len());
        for vtx in validated {
            let (btx, ts, declared_class) =
                vtx.into_blockifier_for_sequencing().map_err(|e| BatchSubmitError::Internal(e.into()))?;
            batch.push(btx, AdditionalTxInfo { declared_class, arrived_at: ts });
        }

        // Serialize batches: hold the lock across send + await so only one batch is in flight.
        let _guard = self.batch_lock.lock().await;
        let (response_tx, response_rx) = oneshot::channel();
        self.executor_commands
            .send(ExecutorCommand::ExecuteBatch { batch, response: response_tx })
            .map_err(|_| BatchSubmitError::Internal(anyhow::anyhow!("Block production executor is not running")))?;
        response_rx
            .await
            .map_err(|_| BatchSubmitError::Internal(anyhow::anyhow!("Executor dropped the batch before completion")))
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
    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        None
    }
    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        None
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
    async fn received_transaction(&self, _hash: starknet_types_core::felt::Felt) -> Option<bool> {
        None
    }
    async fn subscribe_new_transactions(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<starknet_types_core::felt::Felt>> {
        None
    }
}
