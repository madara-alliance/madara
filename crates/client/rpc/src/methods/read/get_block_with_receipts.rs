use dp_convert::ToFelt;
use dp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{
    BlockId, BlockStatus, BlockTag, BlockWithReceipts, MaybePendingBlockWithReceipts, PendingBlockWithReceipts,
    TransactionFinalityStatus, TransactionWithReceipt,
};

use crate::utils::block::{l1_da_mode, l1_data_gas_price, l1_gas_price, starknet_version};
use crate::Starknet;

pub fn get_block_with_receipts(starknet: &Starknet, block_id: BlockId) -> RpcResult<MaybePendingBlockWithReceipts> {
    let block = starknet.get_block(block_id)?;

    let block_txs_hashes = block.tx_hashes().iter().map(ToFelt::to_felt);

    // create a vector of transactions with their corresponding hashes without deploy transactions,
    // blockifier does not support deploy transactions
    let transaction_with_hash: Vec<_> = block.transactions().iter().cloned().zip(block_txs_hashes).collect();

    let transactions_core: Vec<_> = transaction_with_hash
        .iter()
        .cloned()
        .map(|(transaction, hash)| to_starknet_core_tx(&transaction, hash))
        .collect();

    let is_on_l1 = block.block_n() <= starknet.get_l1_last_confirmed_block()?;

    let finality_status =
        if is_on_l1 { TransactionFinalityStatus::AcceptedOnL1 } else { TransactionFinalityStatus::AcceptedOnL2 };

    let receipts: Vec<starknet_core::types::TransactionReceipt> =
        block.receipts().iter().map(|receipt| receipt.clone().to_starknet_core(finality_status)).collect();

    let transactions_with_receipts = transactions_core
        .into_iter()
        .zip(receipts)
        .map(|(transaction, receipt)| TransactionWithReceipt { transaction, receipt })
        .collect();

    let is_pending = matches!(block_id, BlockId::Tag(BlockTag::Pending));

    if is_pending {
        let pending_block_with_receipts = PendingBlockWithReceipts {
            transactions: transactions_with_receipts,
            parent_hash: block.header().parent_block_hash,
            timestamp: block.header().block_timestamp,
            sequencer_address: block.header().sequencer_address,
            l1_gas_price: l1_gas_price(&block),
            l1_data_gas_price: l1_data_gas_price(&block),
            l1_da_mode: l1_da_mode(&block),
            starknet_version: starknet_version(&block),
        };

        let pending_block = MaybePendingBlockWithReceipts::PendingBlock(pending_block_with_receipts);
        Ok(pending_block)
    } else {
        let status = if is_on_l1 { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };

        let block_with_receipts = BlockWithReceipts {
            status,
            block_hash: block.block_hash().to_felt(),
            parent_hash: block.header().parent_block_hash,
            block_number: block.header().block_number,
            new_root: block.header().global_state_root,
            timestamp: block.header().block_timestamp,
            sequencer_address: block.header().sequencer_address,
            l1_gas_price: l1_gas_price(&block),
            l1_data_gas_price: l1_data_gas_price(&block),
            l1_da_mode: l1_da_mode(&block),
            starknet_version: starknet_version(&block),
            transactions: transactions_with_receipts,
        };
        Ok(MaybePendingBlockWithReceipts::Block(block_with_receipts))
    }
}
