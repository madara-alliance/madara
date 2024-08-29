use std::sync::Arc;

use super::providers::AddTransactionProvider;
use crate::errors::StarknetRpcApiError;
use crate::utils::display_internal_server_error;
use dc_mempool::Mempool;
use dc_mempool::MempoolProvider;
use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
};

/// This [`AddTransactionProvider`] adds the received transactions to a mempool.
pub struct MempoolAddTxProvider {
    mempool: Arc<Mempool>,
}

impl MempoolAddTxProvider {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        Self { mempool }
    }
}

fn make_err(err: dc_mempool::Error) -> StarknetRpcApiError {
    if err.is_internal() {
        display_internal_server_error(format!("{err:#}"));
        StarknetRpcApiError::InternalServerError
    } else {
        StarknetRpcApiError::ValidationFailure { error: format!("{err:#}") }
    }
}

#[async_trait]
impl AddTransactionProvider for MempoolAddTxProvider {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        Ok(self.mempool.accept_declare_tx(declare_transaction).map_err(make_err)?)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        Ok(self.mempool.accept_deploy_account_tx(deploy_account_transaction).map_err(make_err)?)
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        Ok(self.mempool.accept_invoke_tx(invoke_transaction).map_err(make_err)?)
    }
}
