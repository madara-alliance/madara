use crate::versions::user::v0_8_1::StarknetWriteRpcApiV0_8_1Server as V0_8_1Impl;
use crate::versions::user::v0_10_1::StarknetWriteRpcApiV0_10_1Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_1::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

// v0.10.1 write API implementation
// Main changes from v0.10.0:
// - BroadcastedInvokeTxnV3 now supports optional proof field
// - The proof is passed to the gateway for transactions that include it

#[async_trait]
impl StarknetWriteRpcApiV0_10_1Server for Starknet {
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
        // TODO: Handle optional proof field in BroadcastedInvokeTxnV3
        // The proof should be extracted and passed to the gateway
        // For now, delegate to v0.8.1 implementation
        V0_8_1Impl::add_invoke_transaction(self, invoke_transaction).await
    }
}
