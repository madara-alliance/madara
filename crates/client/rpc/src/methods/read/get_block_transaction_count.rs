use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::BlockId;

use crate::errors::StarknetRpcApiError;
use crate::utils::get_block_by_block_hash;
use crate::Starknet;

/// Get the Number of Transactions in a Given Block
///
/// ### Arguments
///
/// * `block_id` - The identifier of the requested block. This can be the hash of the block, the
///   block's number (height), or a specific block tag.
///
/// ### Returns
///
/// * `transaction_count` - The number of transactions in the specified block.
///
/// ### Errors
///
/// This function may return a `BLOCK_NOT_FOUND` error if the specified block does not exist in
/// the blockchain.
pub fn get_block_transaction_count<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<u128>
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
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;

    Ok(starknet_block.header().transaction_count)
}
