use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_10_0::{BlockStatus, TxnReceiptWithBlockInfo};
use starknet_types_core::felt::Felt;

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
) -> StarknetRpcResult<TxnReceiptWithBlockInfo> {
    tracing::debug!("get tx receipt {transaction_hash:#x}");
    let view = starknet.backend.view_on_latest();
    let res = view.find_transaction_by_hash(&transaction_hash)?.ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let transaction = res.get_transaction()?;

    let status = if res.block.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if res.block.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };
    let transaction_receipt = transaction.receipt.clone().to_rpc_v0_9(status.into());

    if let Some(confirmed) = res.block.as_confirmed() {
        let block_hash = confirmed.get_block_info()?.block_hash;
        Ok(TxnReceiptWithBlockInfo {
            transaction_receipt,
            block_hash: Some(block_hash),
            block_number: res.block.block_number(),
        })
    } else {
        Ok(TxnReceiptWithBlockInfo { transaction_receipt, block_hash: None, block_number: res.block.block_number() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use rstest::rstest;

    #[rstest]
    fn test_get_transaction_receipt(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, tx_hashes, expected_receipts_v0_9, .. }, rpc) =
            sample_chain_for_block_getters;

        // Block 0
        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_9[0].clone(),
            block_hash: Some(block_hashes[0]),
            block_number: 0,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[0]).unwrap(), res);

        // Block 1

        // Block 2
        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_9[1].clone(),
            block_hash: Some(block_hashes[2]),
            block_number: 2,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[1]).unwrap(), res);
        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_9[2].clone(),
            block_hash: Some(block_hashes[2]),
            block_number: 2,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[2]).unwrap(), res);

        // Pending
        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_9[3].clone(),
            block_hash: None,
            block_number: 3,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[3]).unwrap(), res);
    }

    #[rstest]
    fn test_get_transaction_receipt_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_transaction_receipt(&rpc, does_not_exist), Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
