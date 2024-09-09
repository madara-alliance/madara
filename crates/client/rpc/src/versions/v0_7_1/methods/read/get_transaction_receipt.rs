use mp_block::MadaraMaybePendingBlockInfo;
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
pub fn get_transaction_receipt(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use rstest::rstest;
    use starknet_core::types::ReceiptBlock;

    #[rstest]
    fn test_get_transaction_receipt(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, tx_hashes, expected_receipts, .. }, rpc) =
            sample_chain_for_block_getters;

        // Block 0
        let res = TransactionReceiptWithBlockInfo {
            receipt: expected_receipts[0].clone(),
            block: ReceiptBlock::Block { block_hash: block_hashes[0], block_number: 0 },
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[0]).unwrap(), res);

        // Block 1

        // Block 2
        let res = TransactionReceiptWithBlockInfo {
            receipt: expected_receipts[1].clone(),
            block: ReceiptBlock::Block { block_hash: block_hashes[2], block_number: 2 },
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[1]).unwrap(), res);
        let res = TransactionReceiptWithBlockInfo {
            receipt: expected_receipts[2].clone(),
            block: ReceiptBlock::Block { block_hash: block_hashes[2], block_number: 2 },
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[2]).unwrap(), res);

        // Pending
        let res =
            TransactionReceiptWithBlockInfo { receipt: expected_receipts[3].clone(), block: ReceiptBlock::Pending };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[3]).unwrap(), res);
    }

    #[rstest]
    fn test_get_transaction_receipt_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_transaction_receipt(&rpc, does_not_exist), Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
