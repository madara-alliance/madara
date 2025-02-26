use mp_block::{BlockId, MadaraMaybePendingBlockInfo};
use mp_rpc::{
    BlockHeader, BlockStatus, BlockWithReceipts, PendingBlockHeader, PendingBlockWithReceipts,
    StarknetGetBlockWithTxsAndReceiptsResult, TransactionAndReceipt, TxnFinalityStatus,
};

use crate::errors::StarknetRpcResult;
use crate::Starknet;

pub fn get_block_with_receipts(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
    tracing::debug!("get_block_with_receipts called with {:?}", block_id);
    let block = starknet.get_block(&block_id)?;

    let transactions = block.inner.transactions.into_iter().map(|tx| tx.into());

    let is_on_l1 = if let Some(block_n) = block.info.block_n() {
        block_n <= starknet.get_l1_last_confirmed_block()?
    } else {
        false
    };

    let finality_status = if is_on_l1 { TxnFinalityStatus::L1 } else { TxnFinalityStatus::L2 };

    let receipts = block.inner.receipts.into_iter().map(|receipt| receipt.to_starknet_types(finality_status.clone()));

    let transactions_with_receipts = Iterator::zip(transactions, receipts)
        .map(|(transaction, receipt)| TransactionAndReceipt { receipt, transaction })
        .collect();

    match block.info {
        MadaraMaybePendingBlockInfo::Pending(block) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Pending(PendingBlockWithReceipts {
                transactions: transactions_with_receipts,
                pending_block_header: PendingBlockHeader {
                    parent_hash: block.header.parent_block_hash,
                    timestamp: block.header.block_timestamp.0,
                    sequencer_address: block.header.sequencer_address,
                    l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                    l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                    l1_da_mode: block.header.l1_da_mode.into(),
                    starknet_version: block.header.protocol_version.to_string(),
                },
            }))
        }
        MadaraMaybePendingBlockInfo::NotPending(block) => {
            let status = if is_on_l1 { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
                transactions: transactions_with_receipts,
                status,
                block_header: BlockHeader {
                    block_hash: block.block_hash,
                    parent_hash: block.header.parent_block_hash,
                    block_number: block.header.block_number,
                    new_root: block.header.global_state_root,
                    timestamp: block.header.block_timestamp.0,
                    sequencer_address: block.header.sequencer_address,
                    l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                    l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                    l1_da_mode: block.header.l1_da_mode.into(),
                    starknet_version: block.header.protocol_version.to_string(),
                },
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::StarknetRpcApiError,
        test_utils::{rpc_test_setup, sample_chain_for_block_getters, SampleChainForBlockGetters},
    };
    use mc_db::MadaraBackend;
    use mp_block::{
        header::{BlockTimestamp, GasPrices},
        BlockTag, Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock,
    };
    use mp_chain_config::StarknetVersion;
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::{L1DaMode, ResourcePrice};
    use mp_state_update::StateDiff;
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    #[rstest]
    fn test_get_block_with_receipts(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, expected_txs, expected_receipts, .. }, rpc) =
            sample_chain_for_block_getters;

        // Block 0
        let res = StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
            status: BlockStatus::AcceptedOnL1,
            transactions: vec![TransactionAndReceipt {
                transaction: expected_txs[0].transaction.clone(),
                receipt: expected_receipts[0].clone(),
            }],
            block_header: BlockHeader {
                block_hash: block_hashes[0],
                parent_hash: Felt::ZERO,
                block_number: 0,
                new_root: Felt::from_hex_unchecked("0x88912"),
                timestamp: 43,
                sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                l1_gas_price: ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.1.1".into(),
            },
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
            status: BlockStatus::AcceptedOnL2,
            transactions: vec![],
            block_header: BlockHeader {
                block_hash: block_hashes[1],
                parent_hash: block_hashes[0],
                block_number: 1,
                new_root: Felt::ZERO,
                timestamp: 0,
                sequencer_address: Felt::ZERO,
                l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_da_mode: L1DaMode::Calldata,
                starknet_version: "0.13.2".into(),
            },
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
            status: BlockStatus::AcceptedOnL2,
            transactions: vec![
                TransactionAndReceipt {
                    transaction: expected_txs[1].transaction.clone(),
                    receipt: expected_receipts[1].clone(),
                },
                TransactionAndReceipt {
                    transaction: expected_txs[2].transaction.clone(),
                    receipt: expected_receipts[2].clone(),
                },
            ],
            block_header: BlockHeader {
                block_hash: block_hashes[2],
                parent_hash: block_hashes[1],
                block_number: 2,
                new_root: Felt::ZERO,
                timestamp: 0,
                sequencer_address: Felt::ZERO,
                l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.2".into(),
            },
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);

        // Pending
        let res = StarknetGetBlockWithTxsAndReceiptsResult::Pending(PendingBlockWithReceipts {
            transactions: vec![TransactionAndReceipt {
                transaction: expected_txs[3].transaction.clone(),
                receipt: expected_receipts[3].clone(),
            }],
            pending_block_header: PendingBlockHeader {
                parent_hash: block_hashes[2],
                timestamp: 0,
                sequencer_address: Felt::ZERO,
                l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
                l1_da_mode: L1DaMode::Blob,
                starknet_version: "0.13.2".into(),
            },
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }

    #[rstest]
    fn test_get_block_with_receipts_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(3)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_receipts(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }

    #[rstest]
    fn test_get_block_with_receipts_pending_always_present(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header {
                            parent_block_hash: Felt::ZERO,
                            block_number: 0,
                            transaction_count: 1,
                            global_state_root: Felt::from_hex_unchecked("0x88912"),
                            sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                            block_timestamp: BlockTimestamp(43),
                            transaction_commitment: Felt::from_hex_unchecked("0xbabaa0"),
                            event_count: 0,
                            event_commitment: Felt::from_hex_unchecked("0xb"),
                            state_diff_length: Some(5),
                            state_diff_commitment: Some(Felt::from_hex_unchecked("0xb1")),
                            receipt_commitment: Some(Felt::from_hex_unchecked("0xb4")),
                            protocol_version: StarknetVersion::V0_13_1_1,
                            l1_gas_price: GasPrices {
                                eth_l1_gas_price: 123,
                                strk_l1_gas_price: 12,
                                eth_l1_data_gas_price: 44,
                                strk_l1_data_gas_price: 52,
                            },
                            l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
                        },
                        block_hash: Felt::from_hex_unchecked("0x1777177171"),
                        tx_hashes: vec![Felt::from_hex_unchecked("0x8888888")],
                    }),
                    inner: MadaraBlockInner {
                        transactions: vec![Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                            max_fee: Felt::from_hex_unchecked("0x12"),
                            signature: vec![],
                            contract_address: Felt::from_hex_unchecked("0x4343"),
                            entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                            calldata: vec![Felt::from_hex_unchecked("0x2828")],
                        }))],
                        receipts: vec![TransactionReceipt::Invoke(InvokeTransactionReceipt {
                            transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                            actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                            messages_sent: vec![],
                            events: vec![],
                            execution_resources: ExecutionResources::default(),
                            execution_result: ExecutionResult::Succeeded,
                        })],
                    },
                },
                StateDiff::default(),
                vec![],
                None,
                None,
                None,
            )
            .unwrap();

        let res = StarknetGetBlockWithTxsAndReceiptsResult::Pending(PendingBlockWithReceipts {
            transactions: vec![],
            pending_block_header: PendingBlockHeader {
                parent_hash: Felt::from_hex_unchecked("0x1777177171"),
                sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                timestamp: 43,
                starknet_version: StarknetVersion::V0_13_1_1.to_string(),
                l1_data_gas_price: ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() },
                l1_gas_price: ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() },
                l1_da_mode: L1DaMode::Blob,
            },
        });
        assert_eq!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }
}
