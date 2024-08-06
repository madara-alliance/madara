use dp_block::DeoxysMaybePendingBlockInfo;
use starknet_core::types::{
    BlockId, BlockStatus, BlockWithReceipts, MaybePendingBlockWithReceipts, PendingBlockWithReceipts,
    TransactionFinalityStatus, TransactionWithReceipt,
};

use crate::errors::StarknetRpcResult;
use crate::Starknet;

pub fn get_block_with_receipts(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<MaybePendingBlockWithReceipts> {
    log::debug!("block_id {block_id:?}");
    let block = starknet.get_block(&block_id)?;

    let transactions_core = Iterator::zip(block.inner.transactions.iter(), block.info.tx_hashes())
        .map(|(tx, hash)| tx.clone().to_core(*hash));

    let is_on_l1 = if let Some(block_n) = block.info.block_n() {
        block_n <= starknet.get_l1_last_confirmed_block()?
    } else {
        false
    };

    let finality_status =
        if is_on_l1 { TransactionFinalityStatus::AcceptedOnL1 } else { TransactionFinalityStatus::AcceptedOnL2 };

    let receipts = block.inner.receipts.iter().map(|receipt| receipt.clone().to_starknet_core(finality_status));

    let transactions_with_receipts = Iterator::zip(transactions_core, receipts)
        .map(|(transaction, receipt)| TransactionWithReceipt { transaction, receipt })
        .collect();

    match block.info {
        DeoxysMaybePendingBlockInfo::Pending(block) => {
            Ok(MaybePendingBlockWithReceipts::PendingBlock(PendingBlockWithReceipts {
                transactions: transactions_with_receipts,
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
            let status = if is_on_l1 { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
            Ok(MaybePendingBlockWithReceipts::Block(BlockWithReceipts {
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
                transactions: transactions_with_receipts,
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
    fn test_get_block_with_receipts() {
        let _ = env_logger::builder().is_test(true).try_init();
        let (backend, rpc) = open_testing();
        let SampleChain1 { block_hashes, expected_txs, expected_receipts, .. } = make_sample_chain_1(&backend);

        // Block 0
        let res = MaybePendingBlockWithReceipts::Block(BlockWithReceipts {
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
            transactions: vec![TransactionWithReceipt {
                transaction: expected_txs[0].clone(),
                receipt: expected_receipts[0].clone(),
            }],
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = MaybePendingBlockWithReceipts::Block(BlockWithReceipts {
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
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = MaybePendingBlockWithReceipts::Block(BlockWithReceipts {
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
            transactions: vec![
                TransactionWithReceipt { transaction: expected_txs[1].clone(), receipt: expected_receipts[1].clone() },
                TransactionWithReceipt { transaction: expected_txs[2].clone(), receipt: expected_receipts[2].clone() },
            ],
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);

        // Pending
        let res = MaybePendingBlockWithReceipts::PendingBlock(PendingBlockWithReceipts {
            parent_hash: block_hashes[2],
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.13.2".into(),
            transactions: vec![TransactionWithReceipt {
                transaction: expected_txs[3].clone(),
                receipt: expected_receipts[3].clone(),
            }],
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }
}
