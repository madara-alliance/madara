use crate::versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server as V0_8_1Impl;
use crate::{versions::user::v0_10_0::StarknetWriteRpcApiV0_10_0Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

#[async_trait]
impl StarknetWriteRpcApiV0_10_0Server for Starknet {
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
        V0_8_1Impl::add_invoke_transaction(self, invoke_transaction).await
    }
}
