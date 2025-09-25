use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_convert::Felt;
use mp_rpc::v0_7_1::{
    BlockId, BlockStatus, BlockWithTxHashes, MaybePendingBlockWithTxHashes, PendingBlockWithTxHashes,
};

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
pub fn get_block_with_tx_hashes(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<MaybePendingBlockWithTxHashes> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;
    let is_on_l1 = view.is_on_l1();

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            let parent_hash = if let Some(b) = view.parent_block() {
                b.get_block_info()?.block_hash
            } else {
                Felt::ZERO // genesis
            };
            Ok(MaybePendingBlockWithTxHashes::Pending(PendingBlockWithTxHashes {
                transactions: block.tx_hashes,
                pending_block_header: block.header.to_rpc_v0_7(parent_hash),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            let status = if is_on_l1 { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
            Ok(MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
                transactions: block.tx_hashes.clone(),
                status,
                block_header: block.to_rpc_v0_7(),
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
    use mp_rpc::v0_7_1::{BlockHeader, BlockTag, L1DaMode, PendingBlockHeader, ResourcePrice};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_get_block_with_tx_hashes(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, tx_hashes, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        let res = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
            transactions: vec![tx_hashes[0]],
            status: BlockStatus::AcceptedOnL1,
            block_header: BlockHeader {
                block_hash: block_hashes[0],
                parent_hash: Felt::ZERO,
                block_number: 0,
                new_root: Felt::from_hex_unchecked("0x0"),
                timestamp: 43,
                sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                l1_gas_price: ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.1.1".into(),
            },
        });
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
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
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
            status: BlockStatus::AcceptedOnL2,
            transactions: vec![tx_hashes[1], tx_hashes[2]],
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
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);

        // Pending
        let res = MaybePendingBlockWithTxHashes::Pending(PendingBlockWithTxHashes {
            transactions: vec![tx_hashes[3]],
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
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }

    #[rstest]
    fn test_get_block_with_tx_hashes_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(3)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_tx_hashes(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }
}
