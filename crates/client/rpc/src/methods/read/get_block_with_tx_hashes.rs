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
use starknet_core::types::{BlockId, BlockTag, MaybePendingBlockWithTxHashes};

use crate::errors::StarknetRpcApiError;
use crate::{get_block_with_tx_hashes_finalized, get_block_with_tx_hashes_pending, Starknet, StarknetReadRpcApiServer};

/// Get block information with transaction hashes given the block id.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag.
///
/// ### Returns
///
/// Returns block information with transaction hashes. This includes either a confirmed block or
/// a pending block with transaction hashes, depending on the state of the requested block.
/// In case the block is not found, returns a `StarknetRpcApiError` with `BlockNotFound`.
pub fn get_block_with_tx_hashes<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<MaybePendingBlockWithTxHashes>
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
    let chain_id = starknet.chain_id()?;
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    match block_id {
        BlockId::Tag(BlockTag::Pending) => get_block_with_tx_hashes_pending::<H>(chain_id),
        _ => get_block_with_tx_hashes_finalized(starknet, chain_id, substrate_block_hash),
    }
}
