use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer};
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

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Return the currently configured chain id.
///
/// This function provides the chain id for the network that the node is connected to. The chain
/// id is a unique identifier that distinguishes between different networks, such as mainnet or
/// testnet.
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// Returns the chain id this node is connected to. The chain id is returned as a specific type,
/// defined by the Starknet protocol, indicating the particular network.
#[allow(unused_variables)]
pub fn chain_id<A, B, BE, G, C, P, H>(starknet: &Starknet<A, B, BE, G, C, P, H>) -> RpcResult<Felt>
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
    let best_block_hash = starknet.client.info().best_hash;
    let chain_id = get_config()
        .map_err(|e| {
            error!("Failed to get config: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .chain_id;

    Ok(Felt(chain_id))
}
