use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_rpc::v0_10_0::{BlockStatus, TxnReceiptWithBlockInfo};
use starknet_types_core::felt::Felt;

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
    let transaction_receipt = transaction.receipt.clone().to_rpc_v0_10(status.into());

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
        let (SampleChainForBlockGetters { block_hashes, tx_hashes, expected_receipts_v0_10, .. }, rpc) =
            sample_chain_for_block_getters;

        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_10[0].clone(),
            block_hash: Some(block_hashes[0]),
            block_number: 0,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[0]).unwrap(), res);

        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_10[1].clone(),
            block_hash: Some(block_hashes[2]),
            block_number: 2,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[1]).unwrap(), res);

        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_10[2].clone(),
            block_hash: Some(block_hashes[2]),
            block_number: 2,
        };
        assert_eq!(get_transaction_receipt(&rpc, tx_hashes[2]).unwrap(), res);

        let res = TxnReceiptWithBlockInfo {
            transaction_receipt: expected_receipts_v0_10[3].clone(),
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

    #[rstest]
    fn test_get_transaction_receipt_deserializes_in_starknet_core(
        sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet),
    ) {
        let (SampleChainForBlockGetters { tx_hashes, .. }, rpc) = sample_chain_for_block_getters;

        for tx_hash in [tx_hashes[0], tx_hashes[2], tx_hashes[3]] {
            let receipt = get_transaction_receipt(&rpc, tx_hash).unwrap();
            let value = serde_json::to_value(receipt).unwrap();
            serde_json::from_value::<starknet_core::types::TransactionReceiptWithBlockInfo>(value).unwrap();
        }
    }
}
