use jsonrpsee::core::RpcResult;
use mp_block::{BlockId, MadaraMaybePendingBlockInfo};
use mp_rpc::{
    BlockHeader, BlockStatus, BlockWithTxs, MaybePendingBlockWithTxs, PendingBlockHeader, PendingBlockWithTxs,
    TxnWithHash,
};

use crate::Starknet;

/// Get block information with full transactions given the block id.
///
/// This function retrieves detailed information about a specific block in the StarkNet network,
/// including all transactions contained within that block. The block is identified using its
/// unique block id, which can be the block's hash, its number (height), or a block tag.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter is used to specify the block from which to retrieve information and
///   transactions.
///
/// ### Returns
///
/// Returns detailed block information along with full transactions. Depending on the state of
/// the block, this can include either a confirmed block or a pending block with its
/// transactions. In case the specified block is not found, returns a `StarknetRpcApiError` with
/// `BlockNotFound`.
pub fn get_block_with_txs(starknet: &Starknet, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
    let block = starknet.get_block(&block_id)?;

    let transactions_with_hash = Iterator::zip(block.inner.transactions.into_iter(), block.info.tx_hashes())
        .map(|(transaction, hash)| TxnWithHash { transaction: transaction.into(), transaction_hash: *hash })
        .collect();

    match block.info {
        MadaraMaybePendingBlockInfo::Pending(block) => Ok(MaybePendingBlockWithTxs::Pending(PendingBlockWithTxs {
            transactions: transactions_with_hash,
            pending_block_header: PendingBlockHeader {
                parent_hash: block.header.parent_block_hash,
                timestamp: block.header.block_timestamp.0,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.gas_prices.l1_gas_price(),
                l1_data_gas_price: block.header.gas_prices.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
            },
        })),
        MadaraMaybePendingBlockInfo::NotPending(block) => {
            let status = if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            Ok(MaybePendingBlockWithTxs::Block(BlockWithTxs {
                transactions: transactions_with_hash,
                status,
                block_header: BlockHeader {
                    block_hash: block.block_hash,
                    parent_hash: block.header.parent_block_hash,
                    block_number: block.header.block_number,
                    new_root: block.header.global_state_root,
                    timestamp: block.header.block_timestamp.0,
                    sequencer_address: block.header.sequencer_address,
                    l1_gas_price: block.header.gas_prices.l1_gas_price(),
                    l1_data_gas_price: block.header.gas_prices.l1_data_gas_price(),
                    l1_da_mode: block.header.l1_da_mode.into(),
                    starknet_version: block.header.protocol_version.to_string(),
                },
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::StarknetRpcApiError,
        test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters},
    };
    use mp_block::BlockTag;
    use mp_rpc::{L1DaMode, ResourcePrice};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_get_block_with_txs(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, expected_txs, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        let res = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL1,
            transactions: vec![expected_txs[0].clone()],
            block_header: BlockHeader {
                block_hash: block_hashes[0],
                parent_hash: Felt::ZERO,
                block_number: 0,
                new_root: Felt::from_hex_unchecked("0x88912"),
                timestamp: 43,
                sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                l1_gas_price: ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.1.1".into(),
            },
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL2,
            transactions: vec![],
            block_header: BlockHeader {
                block_hash: block_hashes[1],
                parent_hash: block_hashes[0],
                block_number: 1,
                new_root: Felt::ZERO,
                timestamp: 0,
                sequencer_address: Felt::ZERO,
                l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_da_mode: L1DaMode::Calldata,
                starknet_version: "0.13.2".into(),
            },
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL2,
            transactions: vec![expected_txs[1].clone(), expected_txs[2].clone()],
            block_header: BlockHeader {
                block_hash: block_hashes[2],
                parent_hash: block_hashes[1],
                block_number: 2,
                new_root: Felt::ZERO,
                timestamp: 0,
                sequencer_address: Felt::ZERO,
                l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.2".into(),
            },
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);

        // Pending
        let res = MaybePendingBlockWithTxs::Pending(PendingBlockWithTxs {
            transactions: vec![expected_txs[3].clone()],
            pending_block_header: PendingBlockHeader {
                parent_hash: block_hashes[2],
                timestamp: 0,
                sequencer_address: Felt::ZERO,
                l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.2".into(),
            },
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }

    #[rstest]
    fn test_get_block_with_txs_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(3)), Err(StarknetRpcApiError::BlockNotFound.into()));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_txs(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound.into())
        );
    }
}
