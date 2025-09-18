use crate::executor::{self, ExecutorCommand, ExecutorCommandError};
use crate::util::{AdditionalTxInfo, BatchToExecute, ExecutionStats};
use crate::BatchExecutionResult;
use async_trait::async_trait;
use blockifier::blockifier::transaction_executor::{TransactionExecutionOutput, TransactionExecutorResult};
use blockifier::state::cached_state::{CommitmentStateDiff, StateMaps};
use blockifier::transaction::account_transaction::ExecutionFlags;
use blockifier::transaction::objects::{TransactionExecutionInfo, TransactionExecutionResult};
use mc_db::MadaraBackend;
use mc_submit_tx::{
    SubmitTransaction, SubmitTransactionError, SubmitValidatedTransaction, TransactionValidator,
    TransactionValidatorConfig,
};
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::v0_7_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::validated::ValidatedMempoolTx;
use starknet_api::core::StateDiffCommitment;
use starknet_api::executable_transaction::AccountTransaction;
use starknet_api::state::StateDiff;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

struct BypassInput(mpsc::Sender<ValidatedMempoolTx>);

#[async_trait]
impl SubmitValidatedTransaction for BypassInput {
    async fn submit_validated_transaction(&self, tx: ValidatedMempoolTx) -> Result<(), SubmitTransactionError> {
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
    bypass_input: mpsc::Sender<ValidatedMempoolTx>,
    /// We use TransactionValidator to handle conversion to blockifier, class compilation etc. Mostly for convenience.
    tx_converter: Arc<TransactionValidator>,
}

impl BlockProductionHandle {
    pub(crate) fn new(
        backend: Arc<MadaraBackend>,
        executor_commands: mpsc::UnboundedSender<executor::ExecutorCommand>,
        bypass_input: mpsc::Sender<ValidatedMempoolTx>,
    ) -> Self {
        Self {
            executor_commands,
            bypass_input: bypass_input.clone(),
            tx_converter: TransactionValidator::new(
                Arc::new(BypassInput(bypass_input)),
                backend,
                TransactionValidatorConfig::default().with_disable_validation(true),
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

    pub async fn append_batch(
        &self,
        transactions: Vec<AccountTransaction>,
        transaction_results: Vec<(TransactionExecutionInfo, CommitmentStateDiff)>,
    ) -> Result<(), ExecutorCommandError> {
        let (sender, recv) = oneshot::channel();

        // Create BatchToExecute
        let mut batch_to_execute = BatchToExecute::default();

        // Convert AccountTransaction to Transaction and create AdditionalTxInfo
        for tx in transactions {
            let blockifier_tx = blockifier::transaction::transaction_execution::Transaction::Account(
                blockifier::transaction::account_transaction::AccountTransaction {
                    tx,
                    execution_flags: ExecutionFlags {
                        only_query: false,
                        charge_fee: true,
                        validate: true,
                        strict_nonce_check: true,
                    },
                },
            ); // Assuming From trait is implemented
            let additional_info = AdditionalTxInfo::default(); // We can add declared class if needed
            batch_to_execute.push(blockifier_tx, additional_info);
        }

        // Create blockifier results
        let blockifier_results: Vec<TransactionExecutorResult<TransactionExecutionOutput>> = transaction_results
            .into_iter()
            .map(|(execution_info, state_diff)| {
                // Convert StateDiffCommitment to StateMaps
                let state_maps = StateMaps {
                    nonces: state_diff.address_to_nonce.into_iter().collect(),
                    class_hashes: state_diff.address_to_class_hash.into_iter().collect(),
                    storage: state_diff
                        .storage_updates
                        .into_iter()
                        .flat_map(|(addr, storage_map)| {
                            storage_map.into_iter().map(move |(key, value)| ((addr, key), value))
                        })
                        .collect(),
                    compiled_class_hashes: state_diff.class_hash_to_compiled_class_hash.into_iter().collect(),
                    declared_contracts: HashMap::new(), // Assuming we don't have this for now
                };

                Ok((execution_info, state_maps))
            })
            .collect();

        // Create basic execution stats
        let stats = ExecutionStats {
            n_batches: 1, // Since we're processing one batch
            n_added_to_block: batch_to_execute.len(),
            n_executed: batch_to_execute.len(),
            n_reverted: 0,       // Assuming no reverted transactions
            n_rejected: 0,       // Assuming no rejected transactions
            declared_classes: 0, // Can be updated if we have declare transactions
            l2_gas_consumed: 0,
            exec_duration: Duration::from_secs(0), // Can be set to actual duration if available
        };

        let batch_execution_result: BatchExecutionResult =
            BatchExecutionResult { executed_txs: batch_to_execute, blockifier_results, stats };

        self.executor_commands
            .send(ExecutorCommand::AppendExecutedBatch((batch_execution_result, sender)))
            .map_err(|_| ExecutorCommandError::ChannelClosed)?;
        recv.await.map_err(|_| ExecutorCommandError::ChannelClosed)?
    }

    /// Send a transaction through the bypass channel to bypass mempool and validation.
    pub async fn send_tx_raw(&self, tx: ValidatedMempoolTx) -> Result<(), ExecutorCommandError> {
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
    async fn submit_validated_transaction(&self, tx: ValidatedMempoolTx) -> Result<(), SubmitTransactionError> {
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
