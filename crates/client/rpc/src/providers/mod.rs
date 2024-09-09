pub mod forward_to_provider;
pub mod mempool;

pub use forward_to_provider::*;
pub use mempool::*;

use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
};

#[async_trait]
pub trait AddTransactionProvider: Send + Sync {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult>;

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult>;

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult>;
}
