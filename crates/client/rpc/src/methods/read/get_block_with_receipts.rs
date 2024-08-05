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
    use crate::test_utils::{make_sample_chain_1, open_testing};
    use rstest::rstest;
    use starknet_core::types::{
        BlockTag, ComputationResources, ExecutionResources, ExecutionResult, FeePayment, Felt, InvokeTransaction, InvokeTransactionReceipt, InvokeTransactionV0, L1DataAvailabilityMode, PriceUnit, ResourcePrice, Transaction, TransactionReceipt
    };

    #[rstest]
    fn test_get_block_transaction_count() {
        let (backend, rpc) = open_testing();
        let block_hashes = make_sample_chain_1(&backend);

        // Block 0
        let res = MaybePendingBlockWithReceipts::Block(BlockWithReceipts {
            status: BlockStatus::AcceptedOnL2,
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
                transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                    transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                    max_fee: Felt::from_hex_unchecked("0x12"),
                    signature: vec![],
                    contract_address: Felt::from_hex_unchecked("0x4343"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828")],
                })),
                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                    transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: dp_receipt::ExecutionResources::default().into(),
                    execution_result: ExecutionResult::Succeeded,
                    finality_status: TransactionFinalityStatus::AcceptedOnL2,
                }),
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
        // // Block 2
        // assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), 2);
        // assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(2)).unwrap(), 2);
        // assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), 2);
        // // Pending
        // assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), 1);
    }
}
