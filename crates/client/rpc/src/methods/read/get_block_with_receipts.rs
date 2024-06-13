use dc_db::mapping_db::BlockStorageType;
use dp_convert::felt_wrapper::FeltWrapper;
use dp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::Transaction;
use starknet_core::types::{
    BlockId, BlockStatus, BlockTag, BlockWithReceipts, MaybePendingBlockWithReceipts, PendingBlockWithReceipts,
    TransactionReceipt, TransactionWithReceipt,
};

use super::get_transaction_receipt::receipt;
use crate::utils::block::{l1_da_mode, l1_data_gas_price, l1_gas_price, starknet_version};
use crate::utils::execution::{block_context, re_execute_transactions};
use crate::utils::transaction::blockifier_transactions;
use crate::utils::ResultExt;
use crate::Starknet;

pub fn get_block_with_receipts(starknet: &Starknet, block_id: BlockId) -> RpcResult<MaybePendingBlockWithReceipts> {
    let block = starknet.get_block(block_id)?;
    let block_context = block_context(starknet, block.info())?;

    let block_txs_hashes = block.tx_hashes().iter().map(FeltWrapper::into_field_element);

    // create a vector of transactions with their corresponding hashes without deploy transactions,
    // blockifier does not support deploy transactions
    let transaction_with_hash: Vec<_> = block
        .transactions()
        .iter()
        .cloned()
        .zip(block_txs_hashes)
        .filter(|(tx, _)| !matches!(tx, Transaction::Deploy(_)))
        .collect();

    let transactions_blockifier = blockifier_transactions(starknet, transaction_with_hash.clone())?;

    let execution_infos = re_execute_transactions(starknet, vec![], transactions_blockifier, &block_context)
        .or_internal_server_error("Failed to re-execute transactions")?;

    let transactions_core: Vec<_> = transaction_with_hash
        .iter()
        .cloned()
        .map(|(transaction, hash)| to_starknet_core_tx(&transaction, hash))
        .collect();

    let receipts: Vec<TransactionReceipt> = execution_infos
        .iter()
        .zip(transaction_with_hash)
        .map(|(execution_info, (transaction, transaction_hash))| {
            receipt(
                starknet,
                &transaction,
                execution_info,
                transaction_hash,
                &match block_id {
                    BlockId::Tag(BlockTag::Pending) => BlockStorageType::Pending,
                    _ => BlockStorageType::BlockN(block.block_n()),
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let transactions_with_receipts = transactions_core
        .into_iter()
        .zip(receipts)
        .map(|(transaction, receipt)| TransactionWithReceipt { transaction, receipt })
        .collect();

    let is_pending = matches!(block_id, BlockId::Tag(BlockTag::Pending));

    if is_pending {
        let pending_block_with_receipts = PendingBlockWithReceipts {
            transactions: transactions_with_receipts,
            parent_hash: block.header().parent_block_hash.into_field_element(),
            timestamp: block.header().block_timestamp,
            sequencer_address: block.header().sequencer_address.into_field_element(),
            l1_gas_price: l1_gas_price(&block),
            l1_data_gas_price: l1_data_gas_price(&block),
            l1_da_mode: l1_da_mode(&block),
            starknet_version: starknet_version(&block),
        };

        let pending_block = MaybePendingBlockWithReceipts::PendingBlock(pending_block_with_receipts);
        Ok(pending_block)
    } else {
        let status = if block.block_n() <= starknet.get_l1_last_confirmed_block()? {
            BlockStatus::AcceptedOnL1
        } else {
            BlockStatus::AcceptedOnL2
        };

        let block_with_receipts = BlockWithReceipts {
            status,
            block_hash: block.block_hash().into_field_element(),
            parent_hash: block.header().parent_block_hash.into_field_element(),
            block_number: block.header().block_number,
            new_root: block.header().global_state_root.into_field_element(),
            timestamp: block.header().block_timestamp,
            sequencer_address: block.header().sequencer_address.into_field_element(),
            l1_gas_price: l1_gas_price(&block),
            l1_data_gas_price: l1_data_gas_price(&block),
            l1_da_mode: l1_da_mode(&block),
            starknet_version: starknet_version(&block),
            transactions: transactions_with_receipts,
        };
        Ok(MaybePendingBlockWithReceipts::Block(block_with_receipts))
    }
}
