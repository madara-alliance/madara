use crate::{errors::StarknetRpcResult, Starknet};
use mp_rpc::v0_7_1::BlockId;

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
pub fn get_block_transaction_count(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<u128> {
    let view = starknet.resolve_block_view(block_id)?;
    Ok(view.get_block_info()?.tx_hashes().len() as u128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::StarknetRpcApiError,
        test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters},
    };
    use mp_rpc::v0_7_1::BlockTag;
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_get_block_transaction_count(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Number(0)).unwrap(), 1);
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), 1);
        // Block 1
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Number(1)).unwrap(), 0);
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), 0);
        // Block 2
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Number(2)).unwrap(), 2);
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), 2);
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), 2);
        // Pending
        assert_eq!(get_block_transaction_count(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), 1);
    }

    #[rstest]
    fn test_get_block_transaction_count_not_found(
        sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet),
    ) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_transaction_count(&rpc, BlockId::Number(3)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_transaction_count(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }
}
