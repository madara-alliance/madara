use dp_block::DeoxysMaybePendingBlockInfo;
use starknet_core::types::BlockId;

use crate::{errors::StarknetRpcResult, Starknet};

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
    let block = starknet.get_block_info(&block_id)?;

    let tx_count = match block {
        DeoxysMaybePendingBlockInfo::Pending(block) => block.tx_hashes.len(),
        DeoxysMaybePendingBlockInfo::NotPending(block) => block.header.transaction_count as _,
    };

    Ok(tx_count as _)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{make_sample_chain_1, open_testing, SampleChain1};
    use rstest::rstest;
    use starknet_core::types::BlockTag;

    #[rstest]
    fn test_get_block_transaction_count() {
        let (backend, rpc) = open_testing();
        let SampleChain1 { block_hashes, .. } = make_sample_chain_1(&backend);

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
}
