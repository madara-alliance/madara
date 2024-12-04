use super::AddTransactionProvider;
use crate::{errors::StarknetRpcApiError, utils::display_internal_server_error};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_mempool::Mempool;
use mc_mempool::MempoolProvider;
use mp_transactions::BroadcastedDeclareTransactionV0;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::AddInvokeTransactionResult;
use starknet_types_rpc::{
    BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, ClassAndTxnHash, ContractAndTxnHash,
};
use std::sync::Arc;

/// This [`AddTransactionProvider`] adds the received transactions to a mempool.
pub struct MempoolAddTxProvider {
    mempool: Arc<Mempool>,
}

impl MempoolAddTxProvider {
    pub fn new(mempool: Arc<Mempool>) -> Self {
        Self { mempool }
    }
}

impl From<mc_mempool::Error> for StarknetRpcApiError {
    fn from(value: mc_mempool::Error) -> Self {
        match value {
            mc_mempool::Error::InnerMempool(mc_mempool::TxInsersionError::DuplicateTxn) => {
                StarknetRpcApiError::DuplicateTxn
            }
            mc_mempool::Error::Validation(err) => StarknetRpcApiError::ValidationFailure { error: format!("{err:#}") },
            mc_mempool::Error::InnerMempool(err) => {
                StarknetRpcApiError::ValidationFailure { error: format!("{err:#}") }
            }
            mc_mempool::Error::Exec(err) => {
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
        declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash<Felt>> {
        Ok(self.mempool.accept_declare_v0_tx(declare_v0_transaction).map_err(StarknetRpcApiError::from)?)
    }
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxn<Felt>,
    ) -> RpcResult<ClassAndTxnHash<Felt>> {
        Ok(self.mempool.accept_declare_tx(declare_transaction).map_err(StarknetRpcApiError::from)?)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn<Felt>,
    ) -> RpcResult<ContractAndTxnHash<Felt>> {
        Ok(self.mempool.accept_deploy_account_tx(deploy_account_transaction).map_err(StarknetRpcApiError::from)?)
    }
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn<Felt>,
    ) -> RpcResult<AddInvokeTransactionResult<Felt>> {
        Ok(self.mempool.accept_invoke_tx(invoke_transaction).map_err(StarknetRpcApiError::from)?)
    }
}
