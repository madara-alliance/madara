use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::{
    BlockId, BlockStatus, BlockWithTxs, MaybePreConfirmedBlockWithTxs, PreConfirmedBlockWithTxs, TxnWithHash,
};

pub fn get_block_with_txs(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<MaybePreConfirmedBlockWithTxs> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;

    let transactions_with_hash = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| TxnWithHash {
            transaction: tx.transaction.to_rpc_v0_8(),
            transaction_hash: *tx.receipt.transaction_hash(),
        })
        .collect();

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxs::PreConfirmed(PreConfirmedBlockWithTxs {
                transactions: transactions_with_hash,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxs::Block(BlockWithTxs {
                transactions: transactions_with_hash,
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
    fn test_get_block_with_txs(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, expected_txs_v0_8, .. }, rpc) =
            sample_chain_for_block_getters;

        // Block 0 - verify new v0.10.0 BlockHeader fields
        let result = get_block_with_txs(&rpc, BlockId::Number(0)).unwrap();
        assert_matches!(result, MaybePreConfirmedBlockWithTxs::Block(block) => {
            assert_eq!(block.status, BlockStatus::AcceptedOnL1);
            assert_eq!(block.transactions.len(), 1);
            assert_eq!(block.transactions[0], expected_txs_v0_8[0]);

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
            get_block_with_txs(&rpc, BlockId::Hash(block_hashes[0])).unwrap(),
            get_block_with_txs(&rpc, BlockId::Number(0)).unwrap()
        );

        // Block 2 - verify multiple transactions
        let result = get_block_with_txs(&rpc, BlockId::Number(2)).unwrap();
        assert_matches!(result, MaybePreConfirmedBlockWithTxs::Block(block) => {
            assert_eq!(block.status, BlockStatus::AcceptedOnL2);
            assert_eq!(block.transactions.len(), 2);
            assert_eq!(block.block_header.transaction_count, 2);
        });

        // Preconfirmed
        let result = get_block_with_txs(&rpc, BlockId::Tag(BlockTag::PreConfirmed)).unwrap();
        assert_matches!(result, MaybePreConfirmedBlockWithTxs::PreConfirmed(block) => {
            assert_eq!(block.transactions.len(), 1);
            assert_eq!(block.pre_confirmed_block_header.block_number, 3);
        });
    }

    #[rstest]
    fn test_get_block_with_txs_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(99)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_txs(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }
}
