use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::{
    BlockId, BlockStatus, BlockWithReceipts, PreConfirmedBlockWithReceipts, StarknetGetBlockWithTxsAndReceiptsResult,
    TransactionAndReceipt,
};

pub fn get_block_with_receipts(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    let transactions_with_receipts = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| TransactionAndReceipt {
            receipt: tx.receipt.to_rpc_v0_9(status.into()),
            transaction: tx.transaction.to_rpc_v0_8(),
        })
        .collect();

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::PreConfirmed(PreConfirmedBlockWithReceipts {
                transactions: transactions_with_receipts,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
                transactions: transactions_with_receipts,
                status,
                block_header: block.to_rpc_v0_10(),
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
        FullBlockWithoutCommitments, TransactionWithReceipt,
    };
    use mp_chain_config::StarknetVersion;
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_rpc::v0_10_0::{BlockTag, L1DaMode, PreConfirmedBlockHeader, ResourcePrice};
    use mp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    #[rstest]
    fn test_get_block_with_receipts(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, expected_txs_v0_8, expected_receipts_v0_9, .. }, rpc) =
            sample_chain_for_block_getters;

        // Block 0 - verify new v0.10.0 BlockHeader fields
        let result = get_block_with_receipts(&rpc, BlockId::Number(0)).unwrap();
        assert_matches!(result, StarknetGetBlockWithTxsAndReceiptsResult::Block(block) => {
            assert_eq!(block.status, BlockStatus::AcceptedOnL1);
            assert_eq!(block.transactions.len(), 1);
            assert_eq!(block.transactions[0].transaction, expected_txs_v0_8[0].transaction);
            assert_eq!(block.transactions[0].receipt, expected_receipts_v0_9[0]);

            // Verify existing BlockHeader fields
            assert_eq!(block.block_header.block_hash, block_hashes[0]);
            assert_eq!(block.block_header.parent_hash, Felt::ZERO);
            assert_eq!(block.block_header.block_number, 0);
            assert_eq!(block.block_header.timestamp, 43);
            assert_eq!(block.block_header.sequencer_address, Felt::from_hex_unchecked("0xbabaa"));
            assert_eq!(block.block_header.l1_gas_price, ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() });
            assert_eq!(block.block_header.l1_data_gas_price, ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() });
            assert_eq!(block.block_header.l1_da_mode, L1DaMode::Blob);
            assert_eq!(block.block_header.starknet_version, "0.13.1.1");

            // Verify NEW v0.10.0 BlockHeader fields are present and have expected types
            // These are computed by the database when adding blocks
            assert_eq!(block.block_header.transaction_count, 1);
            assert_eq!(block.block_header.event_count, 0); // No events in test data
            assert_eq!(block.block_header.state_diff_length, 0); // Empty state diff in test data
            // Commitment fields are computed by the database when adding blocks
            // event_commitment is zero when there are no events
            assert_eq!(block.block_header.event_commitment, Felt::ZERO);
            // transaction_commitment and receipt_commitment are non-zero for blocks with transactions
            assert_ne!(block.block_header.transaction_commitment, Felt::ZERO);
            assert_ne!(block.block_header.receipt_commitment, Felt::ZERO);
            // state_diff_commitment is non-zero (even for "empty" state diff due to commitment calculation)
            assert_ne!(block.block_header.state_diff_commitment, Felt::ZERO);
        });

        // Block by hash
        assert_eq!(
            get_block_with_receipts(&rpc, BlockId::Hash(block_hashes[0])).unwrap(),
            get_block_with_receipts(&rpc, BlockId::Number(0)).unwrap()
        );

        // Block 2 - verify multiple transactions
        let result = get_block_with_receipts(&rpc, BlockId::Number(2)).unwrap();
        assert_matches!(result, StarknetGetBlockWithTxsAndReceiptsResult::Block(block) => {
            assert_eq!(block.status, BlockStatus::AcceptedOnL2);
            assert_eq!(block.transactions.len(), 2);
            assert_eq!(block.block_header.block_number, 2);
            assert_eq!(block.block_header.transaction_count, 2);
        });

        // Latest tag
        assert_eq!(
            get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(),
            get_block_with_receipts(&rpc, BlockId::Number(2)).unwrap()
        );

        // Preconfirmed - uses PreConfirmedBlockHeader (no commitment fields)
        let result = get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::PreConfirmed)).unwrap();
        assert_matches!(result, StarknetGetBlockWithTxsAndReceiptsResult::PreConfirmed(block) => {
            assert_eq!(block.transactions.len(), 1);
            assert_eq!(block.pre_confirmed_block_header.block_number, 3);
            assert_eq!(block.pre_confirmed_block_header.l1_da_mode, L1DaMode::Blob);
        });
    }

    #[rstest]
    fn test_get_block_with_receipts_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_receipts(&rpc, BlockId::Number(99)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_receipts(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }

    #[rstest]
    fn test_get_block_with_receipts_preconfirmed_always_present(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;
        let _block_hash = backend
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

        assert_matches!(get_block_with_receipts(&rpc, BlockId::Tag(BlockTag::PreConfirmed)).unwrap(), StarknetGetBlockWithTxsAndReceiptsResult::PreConfirmed(PreConfirmedBlockWithReceipts {
            transactions,
            pre_confirmed_block_header: PreConfirmedBlockHeader {
                block_number: 1,
                sequencer_address,
                timestamp: _,
                starknet_version,
                l1_data_gas_price,
                l1_gas_price,
                l2_gas_price,
                l1_da_mode: L1DaMode::Blob,
            },
        }) => {
            assert_eq!(transactions, vec![]);
            assert_eq!(sequencer_address, Felt::from_hex_unchecked("0xbabaa"));
            assert_eq!(starknet_version, StarknetVersion::V0_13_1_1.to_string());
            assert_eq!(l1_data_gas_price, ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() });
            assert_eq!(l1_gas_price, ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() });
            assert_eq!(l2_gas_price, ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() });
        });
    }
}
