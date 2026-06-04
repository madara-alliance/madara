use crate::versions::user::v0_10_2::{
    BroadcastedTxnBatch, MadaraTxBatchRpcApiV0_10_2Server, StarknetWriteRpcApiV0_10_2Server, TxnBatchEntry,
    TxnBatchExecutionStatus, TxnBatchResult,
};
use crate::versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server as V0_8_1Impl;
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_block_production::TxExecutionOutcome;
use mp_rpc::v0_10_2::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

#[async_trait]
impl StarknetWriteRpcApiV0_10_2Server for Starknet {
    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        V0_8_1Impl::add_declare_transaction(self, declare_transaction).await
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        V0_8_1Impl::add_deploy_account_transaction(self, deploy_account_transaction).await
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        Ok(self
            .add_transaction_provider
            .submit_invoke_transaction(invoke_transaction)
            .await
            .map_err(crate::StarknetRpcApiError::from)?)
    }
}

#[async_trait]
impl MadaraTxBatchRpcApiV0_10_2Server for Starknet {
    async fn add_transaction_batch(&self, batch: BroadcastedTxnBatch) -> RpcResult<TxnBatchResult> {
        // Only available in block production (sequencer) mode.
        let handle = self.block_prod_handle.as_ref().ok_or(StarknetRpcApiError::UnimplementedMethod)?;

        let outcomes = handle.submit_transaction_batch(batch.transactions).await.map_err(StarknetRpcApiError::from)?;

        let transactions = outcomes
            .into_iter()
            .map(|(transaction_hash, outcome)| TxnBatchEntry {
                transaction_hash,
                outcome: match outcome {
                    TxExecutionOutcome::Succeeded => TxnBatchExecutionStatus::Succeeded,
                    TxExecutionOutcome::Reverted(revert_reason) => TxnBatchExecutionStatus::Reverted { revert_reason },
                    TxExecutionOutcome::Rejected(reason) => TxnBatchExecutionStatus::Rejected { reason },
                },
            })
            .collect();

        Ok(TxnBatchResult { transactions })
    }
}
