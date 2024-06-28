use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
};

use super::add_declare_transaction::*;
use super::add_deploy_account_transaction::*;
use super::add_invoke_transaction::*;
use crate::{Starknet, StarknetWriteRpcApiServer};

#[async_trait]
impl StarknetWriteRpcApiServer for Starknet {
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        Ok(add_declare_transaction(self, declare_transaction).await?)
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        Ok(add_deploy_account_transaction(self, deploy_account_transaction).await?)
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        Ok(add_invoke_transaction(self, invoke_transaction).await?)
    }
}
