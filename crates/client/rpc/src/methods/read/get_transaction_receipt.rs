use blockifier::context::BlockContext;
use blockifier::transaction::objects::TransactionExecutionInfo;
use blockifier::transaction::transaction_execution as btx;
use dc_db::mapping_db::BlockStorageType;
use dp_convert::ToFelt;
use dp_convert::ToStarkFelt;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{Felt, TransactionFinalityStatus, TransactionReceiptWithBlockInfo};

use crate::errors::StarknetRpcApiError;
use crate::utils::execution::re_execute_transactions;
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
) -> RpcResult<TransactionReceiptWithBlockInfo> {
    let (block, tx_info) = starknet
        .block_storage()
        .find_tx_hash_block(&TransactionHash(transaction_hash.to_stark_felt()))
        .or_internal_server_error("Error getting block from tx_hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_index = tx_info.tx_index;

    let is_on_l1 = block.block_n() <= starknet.get_l1_last_confirmed_block()?;

    let finality_status =
        if is_on_l1 { TransactionFinalityStatus::AcceptedOnL1 } else { TransactionFinalityStatus::AcceptedOnL2 };

    let receipt = block
        .receipts()
        .get(tx_index)
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?
        .clone()
        .to_starknet_core(finality_status);

    let block = match tx_info.storage_type {
        BlockStorageType::Pending => starknet_core::types::ReceiptBlock::Pending,
        BlockStorageType::BlockN(block_number) => {
            let block_hash = block.block_hash().to_felt();
            starknet_core::types::ReceiptBlock::Block { block_hash, block_number }
        }
    };

    Ok(TransactionReceiptWithBlockInfo { receipt, block })
}

pub(crate) fn execution_infos(
    starknet: &Starknet,
    transactions: Vec<btx::Transaction>,
    block_context: &BlockContext,
) -> RpcResult<TransactionExecutionInfo> {
    // TODO: fix this with vec
    // let (last, prev) = match transactions.split_last() {
    //     Some((last, prev)) => (vec![last.clone()], prev.to_vec()),
    //     None => (transactions, vec![]),
    // };

    let last = transactions;
    let prev = vec![];

    let execution_infos = re_execute_transactions(starknet, prev, last, block_context)
        .map_err(|e| {
            log::error!("Failed to re-execute transactions: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .pop()
        .ok_or_else(|| {
            log::error!("No execution info returned for the last transaction");
            StarknetRpcApiError::InternalServerError
        })?;

    Ok(execution_infos)
}
