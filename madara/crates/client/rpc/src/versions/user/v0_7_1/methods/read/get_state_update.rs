use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::OptionExt;
use crate::utils::ResultExt;
use crate::Starknet;
use mc_db::db_block_id::DbBlockId;
use mp_block::{BlockId, BlockTag};
use mp_rpc::{MaybePendingStateUpdate, PendingStateUpdate, StateUpdate};
use starknet_types_core::felt::Felt;

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
pub fn get_state_update(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<MaybePendingStateUpdate> {
    let resolved_block_id = starknet
        .backend
        .resolve_block_id(&block_id)
        .or_internal_server_error("Error resolving block id")?
        .ok_or(StarknetRpcApiError::BlockNotFound)?;

    let state_diff = starknet
        .backend
        .get_block_state_diff(&resolved_block_id)
        .or_internal_server_error("Error getting contract class hash at")?
        .ok_or(StarknetRpcApiError::BlockNotFound)?;

    match resolved_block_id.is_pending() {
        true => {
            let old_root = if let Some(block) = starknet
                .backend
                .get_block_info(&BlockId::Tag(BlockTag::Latest))
                .or_internal_server_error("Error getting latest block from db")?
            {
                block
                    .as_closed()
                    .ok_or_internal_server_error("Latest block cannot be pending")?
                    .header
                    .global_state_root
            } else {
                // The pending block is actually genesis, so old root is zero (huh?)
                Felt::ZERO
            };
            Ok(MaybePendingStateUpdate::Pending(PendingStateUpdate { old_root, state_diff: state_diff.into() }))
        }
        false => {
            let block_info = &starknet.get_block_info(&resolved_block_id)?;
            let block_info = block_info.as_closed().ok_or_internal_server_error("Block should not be pending")?;

            // Get the old root from the previous block if it exists, otherwise default to zero.
            let old_root = if let Some(val) = block_info.header.block_number.checked_sub(1) {
                let prev_block_info = &starknet.get_block_info(&DbBlockId::Number(val))?;
                let prev_block_info =
                    prev_block_info.as_closed().ok_or_internal_server_error("Block should not be pending")?;

                prev_block_info.header.global_state_root
            } else {
                // for the genesis block, the previous root is zero
                Felt::ZERO
            };

            Ok(MaybePendingStateUpdate::Block(StateUpdate {
                block_hash: block_info.block_hash,
                old_root,
                new_root: block_info.header.global_state_root,
                state_diff: state_diff.into(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use rstest::rstest;

    #[rstest]
    fn test_get_state_update(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { block_hashes, state_roots, state_diffs, .. }, rpc) =
            sample_chain_for_state_updates;

        // Block 0
        let res = MaybePendingStateUpdate::Block(StateUpdate {
            block_hash: block_hashes[0],
            old_root: Felt::ZERO,
            new_root: state_roots[0],
            state_diff: state_diffs[0].clone().into(),
        });
        assert_eq!(get_state_update(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_state_update(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = MaybePendingStateUpdate::Block(StateUpdate {
            block_hash: block_hashes[1],
            old_root: state_roots[0],
            new_root: state_roots[1],
            state_diff: state_diffs[1].clone().into(),
        });
        assert_eq!(get_state_update(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_state_update(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = MaybePendingStateUpdate::Block(StateUpdate {
            block_hash: block_hashes[2],
            old_root: state_roots[1],
            new_root: state_roots[2],
            state_diff: state_diffs[2].clone().into(),
        });
        assert_eq!(get_state_update(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_state_update(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);
        assert_eq!(get_state_update(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);

        // Pending
        let res = MaybePendingStateUpdate::Pending(PendingStateUpdate {
            old_root: state_roots[2],
            state_diff: state_diffs[3].clone().into(),
        });
        assert_eq!(get_state_update(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }

    #[rstest]

    fn test_get_state_update_not_found(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { .. }, rpc) = sample_chain_for_state_updates;

        assert_eq!(get_state_update(&rpc, BlockId::Number(3)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_state_update(&rpc, BlockId::Hash(does_not_exist)), Err(StarknetRpcApiError::BlockNotFound));
    }
}
