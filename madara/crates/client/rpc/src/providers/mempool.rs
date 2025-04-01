use super::AddTransactionProvider;
use crate::{errors::StarknetRpcApiError, utils::display_internal_server_error};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_mempool::Mempool;
use mc_mempool::MempoolProvider;
use mp_rpc::admin::BroadcastedDeclareTxnV0;
use mp_rpc::AddInvokeTransactionResult;
use mp_rpc::{
    BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use std::sync::Arc;

/// This [`AddTransactionProvider`] adds the received transactions to a mempool.
pub struct MempoolAddTxProvider {
    // PERF: this can go as we are always wrapping MempoolAddTxProvider inside
    // Arc<dyn AddMempoolProvider>
    mempool: Arc<Mempool>,
}

impl MempoolAddTxProvider {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        Self { mempool }
    }
}

impl From<mc_mempool::MempoolError> for StarknetRpcApiError {
    fn from(value: mc_mempool::MempoolError) -> Self {
        match value {
            mc_mempool::MempoolError::InnerMempool(mc_mempool::TxInsertionError::DuplicateTxn) => {
                StarknetRpcApiError::DuplicateTxn
            }
            mc_mempool::MempoolError::InnerMempool(mc_mempool::TxInsertionError::Limit(limit)) => {
                StarknetRpcApiError::FailedToReceiveTxn { err: Some(format!("{}", limit).into()) }
            }
            mc_mempool::MempoolError::InnerMempool(mc_mempool::TxInsertionError::NonceConflict) => {
                StarknetRpcApiError::FailedToReceiveTxn {
                    err: Some("A transaction with this nonce and sender address already exists".into()),
                }
            }
            mc_mempool::MempoolError::Validation(err) => {
                StarknetRpcApiError::ValidationFailure { error: format!("{err:#}").into() }
            }
            mc_mempool::MempoolError::Exec(err) => {
                StarknetRpcApiError::TxnExecutionError { tx_index: 0, error: format!("{err:#}") }
            }
            err => {
                display_internal_server_error(format!("{err:#}"));
                StarknetRpcApiError::InternalServerError
            }
        }
    }
}

#[async_trait]
impl AddTransactionProvider for MempoolAddTxProvider {
    async fn add_declare_v0_transaction(
        &self,
        declare_v0_transaction: BroadcastedDeclareTxnV0,
    ) -> RpcResult<ClassAndTxnHash> {
        Ok(self.mempool.tx_accept_declare_v0(declare_v0_transaction).map_err(StarknetRpcApiError::from)?)
    }
    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        Ok(self.mempool.tx_accept_declare(declare_transaction).map_err(StarknetRpcApiError::from)?)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        Ok(self.mempool.tx_accept_deploy_account(deploy_account_transaction).map_err(StarknetRpcApiError::from)?)
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        Ok(self.mempool.tx_accept_invoke(invoke_transaction).map_err(StarknetRpcApiError::from)?)
    }
}
