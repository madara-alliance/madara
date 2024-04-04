use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use log::error;
use mc_db::DeoxysBackend;
use mc_genesis_data_provider::GenesisProvider;
use mc_sync::l2::get_pending_state_update;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::{DBlockT, DHashT};
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::block::BlockHash as APIBlockHash;
use starknet_core::types::{BlockId, BlockTag, FieldElement, MaybePendingStateUpdate, StateDiff, StateUpdate};

use crate::errors::StarknetRpcApiError;
use crate::utils::get_block_by_block_hash;
use crate::Starknet;

fn get_state_update_finalized<A, BE, G, C, P, H>(
    server: &Starknet<A, BE, G, C, P, H>,
    substrate_block_hash: DHashT,
) -> RpcResult<MaybePendingStateUpdate>
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
    let starknet_block = get_block_by_block_hash(server.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>().into();

    let new_root = Felt252Wrapper::from(starknet_block.header().global_state_root).into();

    let old_root = if starknet_block.header().block_number > 0 {
        Felt252Wrapper::from(DeoxysBackend::temporary_global_state_root_getter()).into()
    } else {
        FieldElement::default()
    };

    let state_diff = state_diff(&starknet_block, server)?;

    Ok(MaybePendingStateUpdate::Update(StateUpdate { block_hash, old_root, new_root, state_diff }))
}

fn get_state_update_pending() -> RpcResult<MaybePendingStateUpdate> {
    match get_pending_state_update() {
        Some(state_update) => Ok(MaybePendingStateUpdate::PendingUpdate(state_update)),
        None => Err(Error::Custom("Failed to retrieve pending state update, node not yet synchronized".to_string())),
    }
}

fn state_diff<A, BE, G, C, P, H>(block: &DeoxysBlock, server: &Starknet<A, BE, G, C, P, H>) -> RpcResult<StateDiff>
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
    let starknet_block_hash = APIBlockHash(block.header().hash::<H>().into());

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
pub fn get_state_update<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<MaybePendingStateUpdate>
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

    match block_id {
        BlockId::Tag(BlockTag::Pending) => get_state_update_pending(),
        _ => get_state_update_finalized(starknet, substrate_block_hash),
    }
}
