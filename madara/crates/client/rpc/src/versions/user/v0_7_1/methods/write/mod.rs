use crate::{versions::user::v0_7_1::StarknetWriteRpcApiV0_7_1Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

#[async_trait]
impl StarknetWriteRpcApiV0_7_1Server for Starknet {
    /// Submit a new declare transaction to be added to the chain
    ///
    /// # Arguments
    ///
    /// * `declare_transaction` - the declare transaction to be added to the chain
    ///
    /// # Returns
    ///
    /// * `declare_transaction_result` - the result of the declare transaction
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxn<Felt>,
    ) -> RpcResult<ClassAndTxnHash<Felt>> {
        self.add_transaction_provider.add_declare_transaction(declare_transaction).await
    }

    /// Add an Deploy Account Transaction
    ///
    /// # Arguments
    ///
    /// * `deploy account transaction` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#deploy_account_transaction>
    ///
    /// # Returns
    ///
    /// * `transaction_hash` - transaction hash corresponding to the invocation
    /// * `contract_address` - address of the deployed contract account
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn<Felt>,
    ) -> RpcResult<ContractAndTxnHash<Felt>> {
        self.add_transaction_provider.add_deploy_account_transaction(deploy_account_transaction).await
    }

    /// Add an Invoke Transaction to invoke a contract function
    ///
    /// # Arguments
    ///
    /// * `invoke tx` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#invoke_transaction>
    ///
    /// # Returns
    ///
    /// * `transaction_hash` - transaction hash corresponding to the invocation
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn<Felt>,
    ) -> RpcResult<AddInvokeTransactionResult<Felt>> {
        self.add_transaction_provider.add_invoke_transaction(invoke_transaction).await
    }
}
