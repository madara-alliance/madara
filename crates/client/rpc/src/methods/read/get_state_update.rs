use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mc_rpc_core::utils::get_block_by_block_hash;
pub use mc_rpc_core::StarknetReadRpcApiServer;
use mc_sync::l2::get_pending_state_update;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_api::block::BlockHash;
use starknet_core::types::{BlockId, BlockTag, FieldElement, MaybePendingStateUpdate, StateDiff, StateUpdate};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

pub(crate) fn get_state_update_finalized<A, B, BE, G, C, P, H>(
    server: &Starknet<A, B, BE, G, C, P, H>,
    substrate_block_hash: B::Hash,
) -> RpcResult<MaybePendingStateUpdate>
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
    let starknet_block = get_block_by_block_hash(server.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>().into();

    let new_root = Felt252Wrapper::from(starknet_block.header().global_state_root).into();

    let old_root = if starknet_block.header().block_number > 0 {
        Felt252Wrapper::from(server.backend.temporary_global_state_root_getter()).into()
    } else {
        FieldElement::default()
    };

    let state_diff = state_diff(&starknet_block, server)?;

    Ok(MaybePendingStateUpdate::Update(StateUpdate { block_hash, old_root, new_root, state_diff }))
}

pub(crate) fn get_state_update_pending() -> RpcResult<MaybePendingStateUpdate> {
    match get_pending_state_update() {
        Some(state_update) => Ok(MaybePendingStateUpdate::PendingUpdate(state_update)),
        None => Err(Error::Custom("Failed to retrieve pending state update, node not yet synchronized".to_string())),
    }
}

fn state_diff<A, B, BE, G, C, P, H>(
    block: &mp_block::Block,
    server: &Starknet<A, B, BE, G, C, P, H>,
) -> RpcResult<StateDiff>
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
    let starknet_block_hash = BlockHash(block.header().hash::<H>().into());

    Ok(server.get_state_diff(&starknet_block_hash).map_err(|e| {
        error!("Failed to get state diff. Starknet block hash: {starknet_block_hash}, error: {e}");
        StarknetRpcApiError::InternalServerError
    })?)
}

/// Get the information about the result of executing the requested block.
///
/// This function fetches details about the state update resulting from executing a specific
/// block in the StarkNet network. The block is identified using its unique block id, which can
/// be the block's hash, its number (height), or a block tag.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter specifies the block for which the state update information is
///   required.
///
/// ### Returns
///
/// Returns information about the state update of the requested block, including any changes to
/// the state of the network as a result of the block's execution. This can include a confirmed
/// state update or a pending state update. If the block is not found, returns a
/// `StarknetRpcApiError` with `BlockNotFound`.
#[allow(unused_variables)]
pub fn get_state_update<A, B, BE, G, C, P, H>(
    starknet: &Starknet<A, B, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<MaybePendingStateUpdate>
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
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    match block_id {
        BlockId::Tag(BlockTag::Pending) => get_state_update_pending(),
        _ => get_state_update_finalized(starknet, substrate_block_hash),
    }
}
