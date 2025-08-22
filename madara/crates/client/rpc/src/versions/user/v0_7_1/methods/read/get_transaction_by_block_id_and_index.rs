use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_block::BlockId;
use mp_rpc::TxnWithHash;

/// Get the details of a transaction by a given block id and index.
///
/// This function fetches the details of a specific transaction in the StarkNet network by
/// identifying it through its block and position (index) within that block. If no transaction
/// is found at the specified index, null is returned.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter is used to specify the block in which the transaction is located.
/// * `index` - An integer representing the index in the block where the transaction is expected to
///   be found. The index starts from 0 and increases sequentially for each transaction in the
///   block.
///
/// ### Returns
///
/// Returns the details of the transaction if found, including the transaction hash. The
/// transaction details are returned as a type conforming to the StarkNet protocol. In case of
/// errors like `BLOCK_NOT_FOUND` or `INVALID_TXN_INDEX`, returns a `StarknetRpcApiError`
/// indicating the specific issue.
pub fn get_transaction_by_block_id_and_index(
    starknet: &Starknet,
    block_id: BlockId,
    index: u64,
) -> StarknetRpcResult<TxnWithHash> {
    let view = starknet.backend.block_view(block_id)?;
    let tx = view.get_executed_transaction(index)?.ok_or(StarknetRpcApiError::InvalidTxnIndex)?;

    Ok(TxnWithHash { transaction: tx.transaction.into(), transaction_hash: *tx.receipt.transaction_hash() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use mp_block::BlockTag;
    use rstest::rstest;

    #[rstest]
    fn test_get_transaction_by_block_id_and_index(
        sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet),
    ) {
        let (SampleChainForBlockGetters { block_hashes, expected_txs, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        assert_eq!(get_transaction_by_block_id_and_index(&rpc, BlockId::Number(0), 0).unwrap(), expected_txs[0]);
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Hash(block_hashes[0]), 0).unwrap(),
            expected_txs[0]
        );

        // Block 1

        // Block 2
        assert_eq!(get_transaction_by_block_id_and_index(&rpc, BlockId::Number(2), 0).unwrap(), expected_txs[1]);
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Hash(block_hashes[2]), 0).unwrap(),
            expected_txs[1]
        );
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Tag(BlockTag::Latest), 0).unwrap(),
            expected_txs[1]
        );

        assert_eq!(get_transaction_by_block_id_and_index(&rpc, BlockId::Number(2), 1).unwrap(), expected_txs[2]);
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Hash(block_hashes[2]), 1).unwrap(),
            expected_txs[2]
        );
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Tag(BlockTag::Latest), 1).unwrap(),
            expected_txs[2]
        );

        // Pending
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Tag(BlockTag::Pending), 0).unwrap(),
            expected_txs[3]
        );
    }

    #[rstest]
    fn test_get_transaction_by_block_id_and_index_not_found(
        sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet),
    ) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Number(4), 0),
            Err(StarknetRpcApiError::BlockNotFound)
        );
        assert_eq!(
            get_transaction_by_block_id_and_index(&rpc, BlockId::Number(0), 1),
            Err(StarknetRpcApiError::InvalidTxnIndex)
        );
    }
}
