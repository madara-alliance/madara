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
