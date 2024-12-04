pub mod forward_to_provider;
pub mod mempool;

pub use forward_to_provider::*;
pub use mempool::*;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_transactions::BroadcastedDeclareTransactionV0;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};

#[async_trait]
pub trait AddTransactionProvider: Send + Sync {
    async fn add_declare_v0_transaction(
        &self,
        declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash<Felt>>;
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxn<Felt>,
    ) -> RpcResult<ClassAndTxnHash<Felt>>;

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn<Felt>,
    ) -> RpcResult<ContractAndTxnHash<Felt>>;

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn<Felt>,
    ) -> RpcResult<AddInvokeTransactionResult<Felt>>;
}
