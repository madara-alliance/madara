use dc_db::DeoxysBackend;
use dp_block::{
    chain_config::ChainConfig,
    header::{GasPrices, L1DataAvailabilityMode, PendingHeader},
    DeoxysBlockInfo, DeoxysBlockInner, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, DeoxysPendingBlockInfo,
    Header, StarknetVersion,
};
use dp_receipt::{
    ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
};
use dp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
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

/// Transactions and blocks testing, no state diff, no converted class
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

    {
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
                                entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                                calldata: vec![Felt::from_hex_unchecked("0x2828eeb")],
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
                                    unit: PriceUnit::Fri,
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
    }

    SampleChain1 { block_hashes, tx_hashes, expected_txs, expected_receipts }
}

pub struct SampleChain2 {
    pub block_hashes: Vec<Felt>,
    pub state_roots: Vec<Felt>,
    pub class_hashes: Vec<Felt>,
    pub compiled_class_hashes: Vec<Felt>,
    pub contracts: Vec<Felt>,
    pub keys: Vec<Felt>,
    pub values: Vec<Felt>,
    pub state_diffs: Vec<StateDiff>,
}

/// State diff
pub fn make_sample_chain_2(backend: &DeoxysBackend) -> SampleChain2 {
    let block_hashes = vec![
        Felt::from_hex_unchecked("0x9999999eee"),
        Felt::from_hex_unchecked("0x9999"),
        Felt::from_hex_unchecked("0xffa00abab"),
    ];
    let state_roots = vec![
        Felt::from_hex_unchecked("0xbabababa"),
        Felt::from_hex_unchecked("0xbabababa123"),
        Felt::from_hex_unchecked("0xbabababa123456"),
    ];
    let class_hashes = vec![
        Felt::from_hex_unchecked("0x9100000001"),
        Felt::from_hex_unchecked("0x9100000002"),
        Felt::from_hex_unchecked("0x9100000009"),
    ];
    let compiled_class_hashes = vec![
        Felt::from_hex_unchecked("0x9100000006"),
        Felt::from_hex_unchecked("0x91000000099"),
        Felt::from_hex_unchecked("0x91000000099b"),
    ];
    let contracts = vec![
        Felt::from_hex_unchecked("0x781623786"),
        Felt::from_hex_unchecked("0x78162bbb3786"),
        Felt::from_hex_unchecked("0x7816aaae23786"),
    ];
    let keys = vec![
        Felt::from_hex_unchecked("0x88188"),
        Felt::from_hex_unchecked("0x9981"),
        Felt::from_hex_unchecked("0x9983331"),
    ];
    let values = vec![
        Felt::from_hex_unchecked("0x99918"),
        Felt::from_hex_unchecked("0x1989169"),
        Felt::from_hex_unchecked("0x9981231233331"),
    ];

    let state_diffs = vec![
        StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: contracts[0],
                storage_entries: vec![
                    StorageEntry { key: keys[0], value: values[0] },
                    StorageEntry { key: keys[2], value: values[2] },
                ],
            }],
            deprecated_declared_classes: vec![],
            declared_classes: vec![
                DeclaredClassItem { class_hash: class_hashes[0], compiled_class_hash: compiled_class_hashes[0] },
                DeclaredClassItem { class_hash: class_hashes[1], compiled_class_hash: compiled_class_hashes[1] },
            ],
            deployed_contracts: vec![DeployedContractItem { address: contracts[0], class_hash: class_hashes[0] }],
            replaced_classes: vec![],
            nonces: vec![],
        },
        StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: contracts[0],
                    storage_entries: vec![StorageEntry { key: keys[0], value: values[1] }],
                },
                ContractStorageDiffItem {
                    address: contracts[2],
                    storage_entries: vec![StorageEntry { key: keys[2], value: values[0] }],
                },
            ],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![
                DeployedContractItem { address: contracts[1], class_hash: class_hashes[1] },
                DeployedContractItem { address: contracts[2], class_hash: class_hashes[0] },
            ],
            replaced_classes: vec![],
            nonces: vec![
                NonceUpdate { contract_address: contracts[0], nonce: 1.into() },
                NonceUpdate { contract_address: contracts[2], nonce: 2.into() },
            ],
        },
        StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: contracts[1],
                    storage_entries: vec![StorageEntry { key: keys[0], value: values[0] }],
                },
                ContractStorageDiffItem {
                    address: contracts[2],
                    storage_entries: vec![StorageEntry { key: keys[1], value: values[2] }],
                },
            ],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
        },
        StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: contracts[0],
                storage_entries: vec![
                    StorageEntry { key: keys[1], value: values[0] },
                    StorageEntry { key: keys[0], value: values[2] },
                ],
            }],
            declared_classes: vec![DeclaredClassItem {
                class_hash: class_hashes[2],
                compiled_class_hash: compiled_class_hashes[2],
            }],
            deprecated_declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![ReplacedClassItem { contract_address: contracts[0], class_hash: class_hashes[2] }],
            nonces: vec![
                NonceUpdate { contract_address: contracts[0], nonce: 3.into() },
                NonceUpdate { contract_address: contracts[1], nonce: 2.into() },
            ],
        },
    ];

    {
        // Block 0
        backend
            .store_block(
                DeoxysMaybePendingBlock {
                    info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                        header: Header {
                            parent_block_hash: Felt::ZERO,
                            global_state_root: state_roots[0],
                            block_number: 0,
                            protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[0],
                        tx_hashes: vec![],
                    }),
                    inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[0].clone(),
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
                            global_state_root: state_roots[1],
                            block_number: 1,
                            protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[1],
                        tx_hashes: vec![],
                    }),
                    inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[1].clone(),
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
                            global_state_root: state_roots[2],
                            block_number: 2,
                            protocol_version: StarknetVersion::STARKNET_VERSION_0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[2],
                        tx_hashes: vec![],
                    }),
                    inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[2].clone(),
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
                        tx_hashes: vec![],
                    }),
                    inner: DeoxysBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[3].clone(),
                vec![],
            )
            .unwrap();
    }

    SampleChain2 {
        block_hashes,
        state_roots,
        class_hashes,
        compiled_class_hashes,
        contracts,
        keys,
        values,
        state_diffs,
    }
}
