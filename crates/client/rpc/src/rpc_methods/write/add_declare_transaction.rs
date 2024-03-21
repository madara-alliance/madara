use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetWriteRpcApiServer};
use mc_sync::utility::get_config;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{
    BroadcastedDeclareTransaction,
    DeclareTransactionResult,
};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use crate::Starknet;
use crate::errors::StarknetRpcApiError;

/// Submit a new declare transaction to be added to the chain
///
/// # Arguments
///
/// * `declare_transaction` - the declare transaction to be added to the chain
///
/// # Returns
///
/// * `declare_transaction_result` - the result of the declare transaction
    pub async fn add_declare_transaction<A, B, BE, G, C, P, H>(
        _starknet: &Starknet<A, B, BE, G, C, P, H>,
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