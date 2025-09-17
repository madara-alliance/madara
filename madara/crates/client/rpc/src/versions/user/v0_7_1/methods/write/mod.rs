use crate::{versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server, Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_7_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

#[async_trait]
impl StarknetWriteRpcApiV0_7_1Server for Starknet {
    async fn add_declare_transaction(&self, _declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }
    async fn add_deploy_account_transaction(
        &self,
        _deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }
    async fn add_invoke_transaction(
        &self,
        _invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }
}
