use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::{BlockId, MadaraMaybePreconfirmedBlockInfo};
use mp_convert::Felt;
use mp_rpc::v0_7_1::{
    BlockHeader, BlockStatus, BlockWithReceipts, PendingBlockHeader, PendingBlockWithReceipts,
    StarknetGetBlockWithTxsAndReceiptsResult, TransactionAndReceipt, TxnFinalityStatus,
};

pub fn get_block_with_receipts(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
    let view = starknet.backend.block_view(block_id)?;
    let block_info = view.get_block_info()?;
    let is_on_l1 = view.is_on_l1();
    let finality_status = if is_on_l1 { TxnFinalityStatus::L1 } else { TxnFinalityStatus::L2 };

    let transactions_with_receipts = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| TransactionAndReceipt {
            receipt: tx.receipt.to_starknet_types(finality_status),
            transaction: tx.transaction.into(),
        })
        .collect();

    let parent_hash = if let Some(b) = view.parent_block() {
        b.get_block_info()?.block_hash
    } else {
        Felt::ZERO // genesis
    };

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Pending(PendingBlockWithReceipts {
                transactions: transactions_with_receipts,
                pending_block_header: PendingBlockHeader {
                    parent_hash,
                    timestamp: block.header.block_timestamp.0,
                    sequencer_address: block.header.sequencer_address,
                    l1_gas_price: block.header.gas_prices.l1_gas_price(),
                    l1_data_gas_price: block.header.gas_prices.l1_data_gas_price(),
                    l1_da_mode: block.header.l1_da_mode.into(),
                    starknet_version: block.header.protocol_version.to_string(),
                },
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            let status = if is_on_l1 { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
                transactions: transactions_with_receipts,
                status,
                block_header: BlockHeader {
                    block_hash: block.block_hash,
                    parent_hash,
                    block_number: block.header.block_number,
                    new_root: block.header.global_state_root,
                    timestamp: block.header.block_timestamp.0,
                    sequencer_address: block.header.sequencer_address,
                    l1_gas_price: block.header.gas_prices.l1_gas_price(),
                    l1_data_gas_price: block.header.gas_prices.l1_data_gas_price(),
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
    use assert_matches::assert_matches;
    use mc_db::MadaraBackend;
    use mp_block::{
        header::{BlockTimestamp, GasPrices, PreconfirmedHeader},
        BlockTag, FullBlockWithoutCommitments, TransactionWithReceipt,
    };
    use mp_chain_config::StarknetVersion;
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::v0_7_1::{L1DaMode, ResourcePrice};
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
                new_root: Felt::from_hex_unchecked("0x0"),
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
        let block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                        block_timestamp: BlockTimestamp(43),
                        protocol_version: StarknetVersion::V0_13_1_1,
                        gas_prices: GasPrices {
                            eth_l1_gas_price: 123,
                            strk_l1_gas_price: 12,
                            eth_l1_data_gas_price: 44,
                            strk_l1_data_gas_price: 52,
                            eth_l2_gas_price: 0,
                            strk_l2_gas_price: 0,
                        },
                        l1_da_mode: mp_chain_config::L1DataAvailabilityMode::Blob,
                    },
                    state_diff: Default::default(),
                    transactions: vec![TransactionWithReceipt {
                        transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                            max_fee: Felt::from_hex_unchecked("0x12"),
                            signature: vec![].into(),
                            contract_address: Felt::from_hex_unchecked("0x4343"),
                            entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                            calldata: vec![Felt::from_hex_unchecked("0x2828")].into(),
                        })),
                        receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                            transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                            actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                            messages_sent: vec![],
                            events: vec![],
                            execution_resources: ExecutionResources::default(),
                            execution_result: ExecutionResult::Succeeded,
                        }),
                    }],
                    events: vec![],
                },
                &[],
                true,
            )
            .unwrap()
            .block_hash;

        assert_matches!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), StarknetGetBlockWithTxsAndReceiptsResult::Pending(PendingBlockWithReceipts {
            transactions,
            pending_block_header: PendingBlockHeader {
                parent_hash,
                sequencer_address,
                timestamp: _,
                starknet_version,
                l1_data_gas_price,
                l1_gas_price,
                l1_da_mode: L1DaMode::Blob,
            },
        }) => {
            assert_eq!(parent_hash, block_hash);
            assert_eq!(transactions, vec![]);
            assert_eq!(sequencer_address, Felt::from_hex_unchecked("0xbabaa"));
            assert_eq!(starknet_version, StarknetVersion::V0_13_1_1.to_string());
            assert_eq!(l1_data_gas_price, ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() });
            assert_eq!(l1_gas_price, ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() });
        });
    }
}
