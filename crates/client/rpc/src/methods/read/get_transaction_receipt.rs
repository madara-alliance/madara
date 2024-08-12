use dp_block::MadaraMaybePendingBlockInfo;
use starknet_core::types::{Felt, TransactionFinalityStatus, TransactionReceiptWithBlockInfo};

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};

use crate::utils::ResultExt;
use crate::Starknet;

/// Get the transaction receipt by the transaction hash.
///
/// This function retrieves the transaction receipt for a specific transaction identified by its
/// hash. The transaction receipt includes information about the execution status of the
/// transaction, events generated during its execution, and other relevant details.
///
/// ### Arguments
///
/// * `transaction_hash` - The hash of the requested transaction. This parameter specifies the
///   transaction for which the receipt is requested.
///
/// ### Returns
///
/// Returns a transaction receipt, which can be one of two types:
/// - `TransactionReceipt` if the transaction has been processed and has a receipt.
/// - `PendingTransactionReceipt` if the transaction is pending and the receipt is not yet
///   available.
///
/// ### Errors
///
/// The function may return a `TXN_HASH_NOT_FOUND` error if the specified transaction hash is
/// not found.
pub async fn get_transaction_receipt(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TransactionReceiptWithBlockInfo> {
    let (block, tx_index) = starknet
        .backend
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error getting block from tx_hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let is_on_l1 = if let Some(block_n) = block.info.block_n() {
        block_n <= starknet.get_l1_last_confirmed_block()?
    } else {
        false
    };

    let finality_status =
        if is_on_l1 { TransactionFinalityStatus::AcceptedOnL1 } else { TransactionFinalityStatus::AcceptedOnL2 };

    let receipt = block
        .inner
        .receipts
        .get(tx_index.0 as usize)
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?
        .clone()
        .to_starknet_core(finality_status);

    let block = match block.info {
        MadaraMaybePendingBlockInfo::Pending(_) => starknet_core::types::ReceiptBlock::Pending,
        MadaraMaybePendingBlockInfo::NotPending(block) => starknet_core::types::ReceiptBlock::Block {
            block_hash: block.block_hash,
            block_number: block.header.block_number,
        },
    };

    Ok(TransactionReceiptWithBlockInfo { receipt, block })
}
