use jsonrpsee::core::RpcResult;
use starknet_core::types::{BlockId, BlockTag, Felt, MaybePendingStateUpdate, StateUpdate};

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
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
pub fn get_state_update(starknet: &Starknet, block_id: BlockId) -> RpcResult<MaybePendingStateUpdate> {
    let block = starknet.get_block_info(block_id)?;
    let new_root = block.header().global_state_root;
    let block_hash = *block.block_hash();

    // Get the old root from the previous block if it exists, otherwise default to zero.
    let old_root = if let Some(block_n) = block.block_n().checked_sub(1) {
        let info = starknet.get_block_info(BlockId::Number(block_n))?;
        info.header().global_state_root
    } else {
        Felt::default()
    };

    match block_id {
        BlockId::Tag(BlockTag::Pending) => {
            let state_update = starknet
                .block_storage()
                .get_pending_block_state_update()
                .or_internal_server_error("Failed to get pending state update")?
                .ok_or(StarknetRpcApiError::BlockNotFound)?;
            Ok(MaybePendingStateUpdate::PendingUpdate(state_update))
        }
        _ => {
            let state_diff = starknet
                .backend
                .block_state_diff()
                .get(block.block_n())
                .or_internal_server_error("Failed to get state diff")?
                .ok_or(StarknetRpcApiError::BlockNotFound)?;

            Ok(MaybePendingStateUpdate::Update(StateUpdate { block_hash, old_root, new_root, state_diff }))
        }
    }
}
