use dp_block::DeoxysMaybePendingBlockInfo;
use starknet_core::types::BlockId;

use crate::{errors::StarknetRpcResult, Starknet};

/// Get the Number of Transactions in a Given Block
///
/// ### Arguments
///
/// * `block_id` - The identifier of the requested block. This can be the hash of the block, the
///   block's number (height), or a specific block tag.
///
/// ### Returns
///
/// * `transaction_count` - The number of transactions in the specified block.
///
/// ### Errors
///
/// This function may return a `BLOCK_NOT_FOUND` error if the specified block does not exist in
/// the blockchain.
pub fn get_block_transaction_count(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<u128> {
    let block = starknet.get_block_info(&block_id)?;

    let tx_count = match block {
        DeoxysMaybePendingBlockInfo::Pending(block) => block.tx_hashes.len(),
        DeoxysMaybePendingBlockInfo::NotPending(block) => block.header.transaction_count as _,
    };

    Ok(tx_count as _)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::TestTransactionProvider;
    use dc_db::DeoxysBackend;
    use dp_block::{
        chain_config::ChainConfig, DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock,
        DeoxysMaybePendingBlockInfo, Header,
    };
    use dp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use dp_state_update::StateDiff;
    use dp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction};
    use rstest::rstest;
    use starknet_core::types::Felt;
    use std::sync::Arc;

    #[rstest]
    fn test_get_block_transaction_count() {
        let chain_config = Arc::new(ChainConfig::test_config());
        let backend = DeoxysBackend::open_for_testing(chain_config.clone());
        let starknet = Starknet::new(backend.clone(), chain_config.clone(), Arc::new(TestTransactionProvider));

        backend
            .store_block(
                DeoxysMaybePendingBlock {
                    info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                        header: Header {
                            parent_block_hash: Felt::ZERO,
                            block_number: 0,
                            transaction_count: 1,
                            ..Default::default()
                        },
                        block_hash: Felt::ONE,
                        tx_hashes: vec![Felt::from_hex_unchecked("0x8888888")],
                    }),
                    inner: DeoxysBlockInner {
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
            )
            .unwrap();

        backend
            .store_block(
                DeoxysMaybePendingBlock {
                    info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                        header: Header {
                            parent_block_hash: Felt::ONE,
                            block_number: 1,
                            transaction_count: 0,
                            ..Default::default()
                        },
                        block_hash: Felt::from_hex_unchecked("0xff"),
                        tx_hashes: vec![Felt::from_hex_unchecked("0x848484")],
                    }),
                    inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
                },
                StateDiff::default(),
                vec![],
            )
            .unwrap();

        backend
            .store_block(
                DeoxysMaybePendingBlock {
                    info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                        header: Header {
                            parent_block_hash: Felt::ONE,
                            block_number: 1,
                            transaction_count: 2,
                            ..Default::default()
                        },
                        block_hash: Felt::from_hex_unchecked("0xff3"),
                        tx_hashes: vec![
                            Felt::from_hex_unchecked("0xdd848484"),
                            Felt::from_hex_unchecked("0xdd84848407"),
                        ],
                    }),
                    inner: DeoxysBlockInner {
                        transactions: vec![
                            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                                max_fee: Felt::from_hex_unchecked("0xb12"),
                                signature: vec![],
                                contract_address: Felt::from_hex_unchecked("0x434b3"),
                                entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                                calldata: vec![Felt::from_hex_unchecked("0x2828b")],
                            })),
                            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                                max_fee: Felt::from_hex_unchecked("0xb12"),
                                signature: vec![],
                                contract_address: Felt::from_hex_unchecked("0x434b3"),
                                entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                                calldata: vec![Felt::from_hex_unchecked("0x2828b")],
                            })),
                        ],
                        receipts: vec![
                            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                                transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                                actual_fee: FeePayment {
                                    amount: Felt::from_hex_unchecked("0x94"),
                                    unit: PriceUnit::Wei,
                                },
                                messages_sent: vec![],
                                events: vec![],
                                execution_resources: ExecutionResources::default(),
                                execution_result: ExecutionResult::Succeeded,
                            }),
                            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                                transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                                actual_fee: FeePayment {
                                    amount: Felt::from_hex_unchecked("0x94dd"),
                                    unit: PriceUnit::Wei,
                                },
                                messages_sent: vec![],
                                events: vec![],
                                execution_resources: ExecutionResources::default(),
                                execution_result: ExecutionResult::Reverted { reason: "too bad".into() },
                            }),
                        ],
                    },
                },
                StateDiff::default(),
                vec![],
            )
            .unwrap();

        // Block 0
        assert_eq!(get_block_transaction_count(&starknet, BlockId::Number(0)).unwrap(), 1);
        assert_eq!(get_block_transaction_count(&starknet, BlockId::Hash(Felt::ONE)).unwrap(), 1);
        // Block 1
        assert_eq!(get_block_transaction_count(&starknet, BlockId::Number(1)).unwrap(), 0);
        assert_eq!(get_block_transaction_count(&starknet, BlockId::Hash(Felt::from_hex_unchecked("0xff"))).unwrap(), 0);
        // Block 2
        assert_eq!(get_block_transaction_count(&starknet, BlockId::Number(2)).unwrap(), 2);
        assert_eq!(
            get_block_transaction_count(&starknet, BlockId::Hash(Felt::from_hex_unchecked("0xff3"))).unwrap(),
            0
        );
    }
}
