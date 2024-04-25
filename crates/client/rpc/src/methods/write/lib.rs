use jsonrpsee::core::{async_trait, RpcResult};
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
};

use super::add_declare_transaction::*;
use super::add_deploy_account_transaction::*;
use super::add_invoke_transaction::*;
use crate::{Starknet, StarknetWriteRpcApiServer};

#[async_trait]
impl<BE, C, H> StarknetWriteRpcApiServer for Starknet<BE, C, H>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        add_declare_transaction(self, declare_transaction).await
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        add_deploy_account_transaction(self, deploy_account_transaction).await
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        add_invoke_transaction(self, invoke_transaction).await
    }
}
