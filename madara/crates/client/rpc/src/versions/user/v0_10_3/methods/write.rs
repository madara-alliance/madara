use crate::versions::user::v0_10_2::StarknetWriteRpcApiV0_10_2Server as V0_10_2Impl;
use crate::versions::user::v0_10_3::StarknetWriteRpcApiV0_10_3Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_3::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

// v0.10.3 has no semantic changes to the write API: delegate to v0.10.2.
#[async_trait]
impl StarknetWriteRpcApiV0_10_3Server for Starknet {
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        V0_10_2Impl::add_invoke_transaction(self, invoke_transaction).await
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        V0_10_2Impl::add_deploy_account_transaction(self, deploy_account_transaction).await
    }

    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        V0_10_2Impl::add_declare_transaction(self, declare_transaction).await
    }
}
