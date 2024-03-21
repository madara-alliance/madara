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
use super::lib::*;
use crate::Starknet;

/// Submit a new declare transaction to be added to the chain
///
/// # Arguments
///
/// * `declare_transaction` - the declare transaction to be added to the chain
///
/// # Returns
///
/// * `declare_transaction_result` - the result of the declare transaction
    async fn add_declare_transaction<A, B, BE, G, C, P, H>(
        starknet: &Starknet<A, B, BE, G, C, P, H>,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult>
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
        let config = get_config().map_err(|e| {
            error!("Failed to get config: {e}");
            StarknetRpcApiError::InternalServerError
        })?;
        let sequencer = SequencerGatewayProvider::new(config.feeder_gateway, config.gateway, config.chain_id);

        let sequencer_response = match sequencer.add_declare_transaction(declare_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => {
                error!("Failed to add invoke transaction to sequencer: {e}");
                return Err(StarknetRpcApiError::InternalServerError.into());
            }
        };

        Ok(sequencer_response)
    }