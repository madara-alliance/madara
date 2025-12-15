use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::{
    BlockId, BlockStatus, BlockWithTxHashes, MaybePreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxHashes,
};

pub fn get_block_with_tx_hashes(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<MaybePreConfirmedBlockWithTxHashes> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmed(PreConfirmedBlockWithTxHashes {
                transactions: block.tx_hashes,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            let tx_hashes =
                view.get_executed_transactions(..)?.into_iter().map(|tx| *tx.receipt.transaction_hash()).collect();
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(BlockWithTxHashes {
                transactions: tx_hashes,
                status,
                block_header: block.to_rpc_v0_10(),
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
    use assert_matches::assert_matches;
    use mp_rpc::v0_10_0::{BlockTag, L1DaMode};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_get_block_with_tx_hashes(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, tx_hashes, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0 - verify new v0.10.0 BlockHeader fields
        let result = get_block_with_tx_hashes(&rpc, BlockId::Number(0)).unwrap();
        assert_matches!(result, MaybePreConfirmedBlockWithTxHashes::Block(block) => {
            assert_eq!(block.status, BlockStatus::AcceptedOnL1);
            assert_eq!(block.transactions.len(), 1);
            assert_eq!(block.transactions[0], tx_hashes[0]);

            // Verify BlockHeader fields
            assert_eq!(block.block_header.block_hash, block_hashes[0]);
            assert_eq!(block.block_header.block_number, 0);
            assert_eq!(block.block_header.l1_da_mode, L1DaMode::Blob);

            // Verify NEW v0.10.0 BlockHeader fields
            assert_eq!(block.block_header.transaction_count, 1);
            assert_eq!(block.block_header.event_count, 0);
            assert_eq!(block.block_header.state_diff_length, 0);
        });

        // Block by hash
        assert_eq!(
            get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[0])).unwrap(),
            get_block_with_tx_hashes(&rpc, BlockId::Number(0)).unwrap()
        );

        // Block 2 - verify multiple transactions
        let result = get_block_with_tx_hashes(&rpc, BlockId::Number(2)).unwrap();
        assert_matches!(result, MaybePreConfirmedBlockWithTxHashes::Block(block) => {
            assert_eq!(block.status, BlockStatus::AcceptedOnL2);
            assert_eq!(block.transactions.len(), 2);
            assert_eq!(block.block_header.transaction_count, 2);
        });

        // Preconfirmed
        let result = get_block_with_tx_hashes(&rpc, BlockId::Tag(BlockTag::PreConfirmed)).unwrap();
        assert_matches!(result, MaybePreConfirmedBlockWithTxHashes::PreConfirmed(block) => {
            assert_eq!(block.transactions.len(), 1);
            assert_eq!(block.pre_confirmed_block_header.block_number, 3);
        });
    }

    #[rstest]
    fn test_get_block_with_tx_hashes_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(99)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_tx_hashes(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }
}
