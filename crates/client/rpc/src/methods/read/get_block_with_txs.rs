use starknet_core::types::{BlockId, MaybePendingBlockWithTxs};

use dp_block::DeoxysMaybePendingBlockInfo;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{BlockStatus, BlockWithTxs, PendingBlockWithTxs};

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

    let transactions_core = Iterator::zip(block.inner.transactions.iter(), block.info.tx_hashes())
        .map(|(transaction, hash)| transaction.clone().to_core(*hash))
        .collect();

    match block.info {
        DeoxysMaybePendingBlockInfo::Pending(block) => {
            Ok(MaybePendingBlockWithTxs::PendingBlock(PendingBlockWithTxs {
                transactions: transactions_core,
                parent_hash: block.header.parent_block_hash,
                timestamp: block.header.block_timestamp,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
            }))
        }
        DeoxysMaybePendingBlockInfo::NotPending(block) => {
            let status = if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            Ok(MaybePendingBlockWithTxs::Block(BlockWithTxs {
                status,
                block_hash: block.block_hash,
                parent_hash: block.header.parent_block_hash,
                block_number: block.header.block_number,
                new_root: block.header.global_state_root,
                timestamp: block.header.block_timestamp,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
                transactions: transactions_core,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{make_sample_chain_1, open_testing, SampleChain1};
    use rstest::rstest;
    use starknet_core::types::{BlockTag, Felt, L1DataAvailabilityMode, ResourcePrice};

    #[rstest]
    fn test_get_block_with_txs() {
        let _ = env_logger::builder().is_test(true).try_init();
        let (backend, rpc) = open_testing();
        let SampleChain1 { block_hashes, expected_txs, .. } = make_sample_chain_1(&backend);

        // Block 0
        let res = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL1,
            block_hash: block_hashes[0],
            parent_hash: Felt::ZERO,
            block_number: 0,
            new_root: Felt::from_hex_unchecked("0x88912"),
            timestamp: 43,
            sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
            l1_gas_price: ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() },
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.13.1.1".into(),
            transactions: vec![expected_txs[0].clone()],
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL2,
            block_hash: block_hashes[1],
            parent_hash: block_hashes[0],
            block_number: 1,
            new_root: Felt::ZERO,
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.13.2".into(),
            transactions: vec![],
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = MaybePendingBlockWithTxs::Block(BlockWithTxs {
            status: BlockStatus::AcceptedOnL2,
            block_hash: block_hashes[2],
            parent_hash: block_hashes[1],
            block_number: 2,
            new_root: Felt::ZERO,
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.13.2".into(),
            transactions: vec![expected_txs[1].clone(), expected_txs[2].clone()],
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_block_with_txs(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);

        // Pending
        let res = MaybePendingBlockWithTxs::PendingBlock(PendingBlockWithTxs {
            parent_hash: block_hashes[2],
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.13.2".into(),
            transactions: vec![expected_txs[3].clone()],
        });
        assert_eq!(get_block_with_txs(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }
}
