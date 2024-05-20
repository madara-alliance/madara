use jsonrpsee::core::error::Error;
use jsonrpsee::core::RpcResult;
use mc_db::storage_handler;
use mc_sync::l2::get_pending_state_update;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BlockTag, FieldElement, MaybePendingStateUpdate, StateUpdate};

use crate::deoxys_backend_client::get_block_by_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::Starknet;

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
pub fn get_state_update<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    block_id: BlockId,
) -> RpcResult<MaybePendingStateUpdate>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> ,
    H: HasherT + Send + Sync + 'static,
{
    match block_id {
        BlockId::Tag(BlockTag::Pending) => get_state_update_pending(),
        _ => get_state_update_finalized(starknet, block_id),
    }
}

fn get_state_update_finalized<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    block_id: BlockId,
) -> RpcResult<MaybePendingStateUpdate>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> ,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id)?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;

    let block_hash = starknet_block.header().hash::<H>().into();

    let block_number = starknet_block.header().block_number;

    let new_root = Felt252Wrapper::from(starknet_block.header().global_state_root).into();

    // Get the old root from the previous block if it exists, otherwise default to zero.
    let old_root = if starknet_block.header().block_number > 0 {
        let previous_substrate_block_hash =
            starknet.substrate_block_hash_from_starknet_block(BlockId::Number(block_number - 1))?;
        let previous_starknet_block = get_block_by_block_hash(starknet.client.as_ref(), previous_substrate_block_hash)?;
        Felt252Wrapper::from(previous_starknet_block.header().global_state_root).into()
    } else {
        FieldElement::default()
    };

    let state_diff = storage_handler::block_state_diff()
        .get(block_number)
        .map_err(|e| {
            log::error!("Failed to get state diff: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .ok_or(StarknetRpcApiError::BlockNotFound)?;

    Ok(MaybePendingStateUpdate::Update(StateUpdate { block_hash, old_root, new_root, state_diff }))
}

fn get_state_update_pending() -> RpcResult<MaybePendingStateUpdate> {
    match get_pending_state_update() {
        Some(state_update) => Ok(MaybePendingStateUpdate::PendingUpdate(state_update)),
        None => Err(Error::Custom("Failed to retrieve pending state update, node not yet synchronized".to_string())),
    }
}
