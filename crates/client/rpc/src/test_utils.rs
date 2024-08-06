use dc_db::{block_db::ChainInfo, DeoxysBackend};
use dp_block::{
    header::{GasPrices, L1DataAvailabilityMode, PendingHeader},
    DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, DeoxysPendingBlockInfo,
    Header, StarknetVersion,
};
use dp_receipt::{
    ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
};
use dp_state_update::StateDiff;
use dp_transactions::{InvokeTransaction, InvokeTransactionV0, Transaction};
use starknet_core::types::Felt;
use std::sync::Arc;

use crate::{providers::TestTransactionProvider, Starknet};

pub fn open_testing() -> (Arc<DeoxysBackend>, Starknet) {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = DeoxysBackend::open_for_testing(chain_config.clone());
    let rpc = Starknet::new(backend.clone(), chain_config.clone(), Arc::new(TestTransactionProvider));

    (backend, rpc)
}

pub struct SampleChain1 {
    pub block_hashes: Vec<Felt>,
    pub tx_hashes: Vec<Felt>,
    pub expected_txs: Vec<starknet_core::types::Transaction>,
    pub expected_receipts: Vec<starknet_core::types::TransactionReceipt>,
}

pub fn make_sample_chain_1(backend: &DeoxysBackend) -> SampleChain1 {
    let block_hashes = vec![Felt::ONE, Felt::from_hex_unchecked("0xff"), Felt::from_hex_unchecked("0xffabab")];
    let tx_hashes = vec![
        Felt::from_hex_unchecked("0x8888888"),
        Felt::from_hex_unchecked("0xdd848484"),
        Felt::from_hex_unchecked("0xdd84848407"),
        Felt::from_hex_unchecked("0xdd84847784"),
    ];
    let expected_txs = {
        use starknet_core::types::{InvokeTransaction, InvokeTransactionV0, Transaction};
        vec![
            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                max_fee: Felt::from_hex_unchecked("0x12"),
                signature: vec![],
                contract_address: Felt::from_hex_unchecked("0x4343"),
                entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                calldata: vec![Felt::from_hex_unchecked("0x2828")],
            })),
            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                max_fee: Felt::from_hex_unchecked("0xb12"),
                signature: vec![],
                contract_address: Felt::from_hex_unchecked("0x434b3"),
                entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                calldata: vec![Felt::from_hex_unchecked("0x2828b")],
            })),
            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                max_fee: Felt::from_hex_unchecked("0xb12"),
                signature: vec![],
                contract_address: Felt::from_hex_unchecked("0x434b3"),
                entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                calldata: vec![Felt::from_hex_unchecked("0x2828eeb")],
            })),
            Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                max_fee: Felt::from_hex_unchecked("0xb12"),
                signature: vec![],
                contract_address: Felt::from_hex_unchecked("0x434b3"),
                entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                calldata: vec![Felt::from_hex_unchecked("0x2828b")],
            })),
        ]
    };
    let expected_receipts = {
        use starknet_core::types::{
            ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionFinalityStatus,
            TransactionReceipt,
        };
        vec![
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                messages_sent: vec![],
                events: vec![],
                execution_resources: dp_receipt::ExecutionResources::default().into(),
                execution_result: ExecutionResult::Succeeded,
                finality_status: TransactionFinalityStatus::AcceptedOnL1,
            }),
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                messages_sent: vec![],
                events: vec![],
                execution_resources: dp_receipt::ExecutionResources::default().into(),
                execution_result: ExecutionResult::Succeeded,
                finality_status: TransactionFinalityStatus::AcceptedOnL2,
            }),
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94dd"), unit: PriceUnit::Fri },
                messages_sent: vec![],
                events: vec![],
                execution_resources: dp_receipt::ExecutionResources::default().into(),
                execution_result: ExecutionResult::Reverted { reason: "too bad".into() },
                finality_status: TransactionFinalityStatus::AcceptedOnL2,
            }),
            TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                messages_sent: vec![],
                events: vec![],
                execution_resources: dp_receipt::ExecutionResources::default().into(),
                execution_result: ExecutionResult::Succeeded,
                finality_status: TransactionFinalityStatus::AcceptedOnL2,
            }),
        ]
    };

    // Block 0
    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                    header: Header {
                        parent_block_hash: Felt::ZERO,
                        block_number: 0,
                        transaction_count: 1,
                        global_state_root: Felt::from_hex_unchecked("0x88912"),
                        sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
                        block_timestamp: 43,
                        transaction_commitment: Felt::from_hex_unchecked("0xbabaa0"),
                        event_count: 0,
                        event_commitment: Felt::from_hex_unchecked("0xb"),
                        state_diff_length: 5,
                        state_diff_commitment: Felt::from_hex_unchecked("0xb1"),
                        receipt_commitment: Felt::from_hex_unchecked("0xb4"),
                        protocol_version: StarknetVersion::STARKNET_VERSION_0_13_1_1,
                        l1_gas_price: GasPrices {
                            eth_l1_gas_price: 123,
                            strk_l1_gas_price: 12,
                            eth_l1_data_gas_price: 44,
                            strk_l1_data_gas_price: 52,
                        },
                        l1_da_mode: L1DataAvailabilityMode::Blob,
                    },
                    block_hash: block_hashes[0],
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

    // Block 1
    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                    header: Header {
                        parent_block_hash: block_hashes[0],
                        block_number: 1,
                        transaction_count: 0,
                        l1_da_mode: L1DataAvailabilityMode::Calldata,
                        protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
                        ..Default::default()
                    },
                    block_hash: block_hashes[1],
                    tx_hashes: vec![],
                }),
                inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
            },
            StateDiff::default(),
            vec![],
        )
        .unwrap();

    // Block 2
    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                    header: Header {
                        parent_block_hash: block_hashes[1],
                        block_number: 2,
                        transaction_count: 2,
                        protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
                        ..Default::default()
                    },
                    block_hash: block_hashes[2],
                    tx_hashes: vec![Felt::from_hex_unchecked("0xdd848484"), Felt::from_hex_unchecked("0xdd84848407")],
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
                            entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                            calldata: vec![Felt::from_hex_unchecked("0x2828eeb")],
                        })),
                    ],
                    receipts: vec![
                        TransactionReceipt::Invoke(InvokeTransactionReceipt {
                            transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                            actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                            messages_sent: vec![],
                            events: vec![],
                            execution_resources: ExecutionResources::default(),
                            execution_result: ExecutionResult::Succeeded,
                        }),
                        TransactionReceipt::Invoke(InvokeTransactionReceipt {
                            transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                            actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94dd"), unit: PriceUnit::Fri },
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

    // Pending
    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::Pending(DeoxysPendingBlockInfo {
                    header: PendingHeader {
                        parent_block_hash: block_hashes[2],
                        protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
                        ..Default::default()
                    },
                    tx_hashes: vec![Felt::from_hex_unchecked("0xdd84847784")],
                }),
                inner: DeoxysBlockInner {
                    transactions: vec![Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                        max_fee: Felt::from_hex_unchecked("0xb12"),
                        signature: vec![],
                        contract_address: Felt::from_hex_unchecked("0x434b3"),
                        entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                        calldata: vec![Felt::from_hex_unchecked("0x2828b")],
                    }))],
                    receipts: vec![TransactionReceipt::Invoke(InvokeTransactionReceipt {
                        transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                        actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
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

    SampleChain1 { block_hashes, tx_hashes, expected_txs, expected_receipts }
}
