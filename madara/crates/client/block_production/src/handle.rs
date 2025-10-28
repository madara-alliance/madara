use crate::executor::{self, ExecutorCommand, ExecutorCommandError};
use async_trait::async_trait;
use mc_db::MadaraBackend;
use mc_submit_tx::{
    SubmitTransaction, SubmitTransactionError, SubmitValidatedTransaction, TransactionValidator,
    TransactionValidatorConfig,
};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::validated::ValidatedTransaction;
use std::sync::Arc;
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

#[derive(Clone, Debug)]
/// Remotely control block production.
pub struct BlockProductionHandle {
    /// Commands to executor task.
    executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
    bypass_input: mpsc::Sender<ValidatedTransaction>,
    /// We use TransactionValidator to handle conversion to blockifier, class compilation etc. Mostly for convenience.
    tx_converter: Arc<TransactionValidator>,
}

impl BlockProductionHandle {
    pub(crate) fn new(
        backend: Arc<MadaraBackend>,
        executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
        bypass_input: mpsc::Sender<ValidatedTransaction>,
        no_charge_fee: bool
    ) -> Self {
        Self {
            executor_commands,
            bypass_input: bypass_input.clone(),
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

    /// Send a transaction through the bypass channel to bypass mempool and validation.
    pub async fn send_tx_raw(&self, tx: ValidatedTransaction) -> Result<(), ExecutorCommandError> {
        self.bypass_input.send(tx).await.map_err(|_| ExecutorCommandError::ChannelClosed)
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
