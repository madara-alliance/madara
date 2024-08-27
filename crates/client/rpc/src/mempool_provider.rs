use std::sync::Arc;

use super::providers::AddTransactionProvider;
use crate::{bail_internal_server_error, errors::StarknetRpcApiError};
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use dc_mempool::Mempool;
use dp_class::ConvertedClass;
use dp_transactions::broadcasted_to_blockifier;
use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    BroadcastedTransaction, DeclareTransactionResult, DeployAccountTransactionResult, Felt, InvokeTransactionResult,
};

/// This [`AddTransactionProvider`] adds the received transactions to a mempool.
pub struct MempoolProvider {
    mempool: Arc<Mempool>,
}

impl MempoolProvider {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        Self { mempool }
    }
}

#[async_trait]
impl AddTransactionProvider for MempoolProvider {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        Ok(self.mempool.accept_declare_tx(declare_transaction).await?)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        Ok(self.mempool.accept_deploy_account_tx(deploy_account_transaction).await?)
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        Ok(self.mempool.accept_invoke_tx(invoke_transaction).await?)
    }
}