use std::marker::PhantomData;
use std::sync::Arc;

use errors::StarknetRpcApiError;
use jsonrpsee::core::{async_trait, RpcResult};
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetWriteRpcApiServer};
use mc_storage::OverrideHandle;
use mc_sync::utility::get_config;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
pub use rpc_methods::spec_version::SpecVersionServer;
pub use rpc_methods::syncing::SyncingServer;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_network_sync::SyncingService;
use sc_transaction_pool::{ChainApi, Pool};
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_arithmetic::traits::UniqueSaturatedInto;
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkHash;
use starknet_core::types::{
    BlockId, BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult, StateDiff,
};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use super::add_declare_transaction::*;
use super::add_deploy_account_transaction::*;
use super::add_invoke_transaction::*;


#[async_trait]
impl<A, B, BE, G, C, P, H> StarknetWriteRpcApiServer for Starknet<A, B, BE, G, C, P, H>
where
    A: ChainApi<Block = B> + 'static,
    B: BlockT,
    P: TransactionPool<Block = B> + 'static,
    BE: Backend<B> + 'static,
    C: HeaderBackend<B> + BlockBackend<B> + StorageProvider<B, BE> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B> + ConvertTransactionRuntimeApi<B>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult>  {
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