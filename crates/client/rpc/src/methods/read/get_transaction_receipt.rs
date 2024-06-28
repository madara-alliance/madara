use dc_db::mapping_db::BlockStorageType;
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
    let (block, tx_info) = starknet
        .block_storage()
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error getting block from tx_hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_index = tx_info.tx_index;

    let finality_status = if block.block_n() <= starknet.get_l1_last_confirmed_block()? {
        TransactionFinalityStatus::AcceptedOnL1
    } else {
        TransactionFinalityStatus::AcceptedOnL2
    };

    let receipt = block
        .receipts()
        .get(tx_index)
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?
        .clone()
        .to_starknet_core(finality_status);

    let block = match tx_info.storage_type {
        BlockStorageType::Pending => starknet_core::types::ReceiptBlock::Pending,
        BlockStorageType::BlockN(block_number) => {
            let block_hash = *block.block_hash();
            starknet_core::types::ReceiptBlock::Block { block_hash, block_number }
        }
    };

    Ok(TransactionReceiptWithBlockInfo { receipt, block })
}
