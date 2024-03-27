use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mc_sync::utility::get_config;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BroadcastedDeclareTransaction, DeclareTransactionResult};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::errors::StarknetRpcApiError;
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
pub async fn add_declare_transaction<A, BE, G, C, P, H>(
    _starknet: &Starknet<A, BE, G, C, P, H>,
    declare_transaction: BroadcastedDeclareTransaction,
) -> RpcResult<DeclareTransactionResult>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
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
