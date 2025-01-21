use jsonrpsee::core::{async_trait, RpcResult};
use mc_db::MadaraBackend;
use mp_block::{
    header::{BlockTimestamp, GasPrices, L1DataAvailabilityMode, PendingHeader},
    Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo,
    MadaraPendingBlockInfo,
};
use mp_chain_config::{ChainConfig, StarknetVersion};
use mp_receipt::{
    ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use mp_transactions::{BroadcastedDeclareTransactionV0, InvokeTransaction, InvokeTransactionV0, Transaction};
use mp_utils::service::ServiceContext;
use rstest::fixture;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash, TxnReceipt, TxnWithHash,
};
use std::sync::Arc;

use crate::{providers::AddTransactionProvider, Starknet};

#[cfg(test)]
pub struct TestTransactionProvider;

#[cfg(test)]
#[async_trait]
impl AddTransactionProvider for TestTransactionProvider {
    async fn add_declare_v0_transaction(
        &self,
        _declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash<Felt>> {
        unimplemented!()
    }
    async fn add_declare_transaction(
        &self,
        _declare_transaction: BroadcastedDeclareTxn<Felt>,
    ) -> RpcResult<ClassAndTxnHash<Felt>> {
        unimplemented!()
    }
    async fn add_deploy_account_transaction(
        &self,
        _deploy_account_transaction: BroadcastedDeployAccountTxn<Felt>,
    ) -> RpcResult<ContractAndTxnHash<Felt>> {
        unimplemented!()
    }
    async fn add_invoke_transaction(
        &self,
        _invoke_transaction: BroadcastedInvokeTxn<Felt>,
    ) -> RpcResult<AddInvokeTransactionResult<Felt>> {
        unimplemented!()
    }
}

#[fixture]
pub fn rpc_test_setup() -> (Arc<MadaraBackend>, Starknet) {
    let chain_config = Arc::new(ChainConfig::madara_test());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let rpc = Starknet::new(
        backend.clone(),
        Arc::new(TestTransactionProvider),
        Default::default(),
        ServiceContext::new_for_testing(),
    );
    (backend, rpc)
}

// This sample chain is only used to test get tx / get block rpcs.
pub struct SampleChainForBlockGetters {
    pub block_hashes: Vec<Felt>,
    pub tx_hashes: Vec<Felt>,
    pub expected_txs: Vec<starknet_types_rpc::TxnWithHash<Felt>>,
    pub expected_receipts: Vec<TxnReceipt<Felt>>,
}

#[fixture]
pub fn sample_chain_for_block_getters(
    rpc_test_setup: (Arc<MadaraBackend>, Starknet),
) -> (SampleChainForBlockGetters, Starknet) {
    let (backend, rpc) = rpc_test_setup;
    (make_sample_chain_for_block_getters(&backend), rpc)
}

/// Transactions and blocks testing, no state diff, no converted class
pub fn make_sample_chain_for_block_getters(backend: &MadaraBackend) -> SampleChainForBlockGetters {
    let block_hashes = vec![Felt::ONE, Felt::from_hex_unchecked("0xff"), Felt::from_hex_unchecked("0xffabab")];
    let tx_hashes = vec![
        Felt::from_hex_unchecked("0x8888888"),
        Felt::from_hex_unchecked("0xdd848484"),
        Felt::from_hex_unchecked("0xdd84848407"),
        Felt::from_hex_unchecked("0xdd84847784"),
    ];
    let expected_txs = {
        use starknet_types_rpc::{InvokeTxn, InvokeTxnV0, Txn};
        vec![
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0x12"),
                    signature: vec![],
                    contract_address: Felt::from_hex_unchecked("0x4343"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828")],
                })),
                transaction_hash: tx_hashes[0],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![],
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828b")],
                })),
                transaction_hash: tx_hashes[1],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![],
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828eeb")],
                })),
                transaction_hash: tx_hashes[2],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![],
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828b")],
                })),
                transaction_hash: tx_hashes[3],
            },
        ]
    };
    let expected_receipts = {
        use starknet_types_rpc::{
            CommonReceiptProperties, FeePayment, InvokeTxnReceipt, PriceUnit, TxnFinalityStatus, TxnReceipt,
        };
        vec![
            TxnReceipt::<Felt>::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources(),
                    finality_status: TxnFinalityStatus::L1,
                    execution_status: starknet_types_rpc::ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::<Felt>::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: starknet_types_rpc::ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::<Felt>::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94dd"), unit: PriceUnit::Fri },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: starknet_types_rpc::ExecutionStatus::Reverted("too bad".into()),
                },
            }),
            TxnReceipt::<Felt>::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: starknet_types_rpc::ExecutionStatus::Successful,
                },
            }),
        ]
    };

    {
        // Block 0
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
                            l1_da_mode: L1DataAvailabilityMode::Blob,
                        },
                        block_hash: block_hashes[0],
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

        // Block 1
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header {
                            parent_block_hash: block_hashes[0],
                            block_number: 1,
                            transaction_count: 0,
                            l1_da_mode: L1DataAvailabilityMode::Calldata,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[1],
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                StateDiff::default(),
                vec![],
                None,
                None,
                None,
            )
            .unwrap();

        // Block 2
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header {
                            parent_block_hash: block_hashes[1],
                            block_number: 2,
                            transaction_count: 2,
                            l1_da_mode: L1DataAvailabilityMode::Blob,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[2],
                        tx_hashes: vec![
                            Felt::from_hex_unchecked("0xdd848484"),
                            Felt::from_hex_unchecked("0xdd84848407"),
                        ],
                    }),
                    inner: MadaraBlockInner {
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
                None,
                None,
                None,
            )
            .unwrap();

        // Pending
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                        header: PendingHeader {
                            parent_block_hash: block_hashes[2],
                            protocol_version: StarknetVersion::V0_13_2,
                            l1_da_mode: L1DataAvailabilityMode::Blob,
                            ..Default::default()
                        },
                        tx_hashes: vec![Felt::from_hex_unchecked("0xdd84847784")],
                    }),
                    inner: MadaraBlockInner {
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
                None,
                None,
                None,
            )
            .unwrap();
    }

    SampleChainForBlockGetters { block_hashes, tx_hashes, expected_txs, expected_receipts }
}

fn defaut_execution_resources() -> starknet_types_rpc::ExecutionResources {
    starknet_types_rpc::ExecutionResources {
        bitwise_builtin_applications: None,
        ec_op_builtin_applications: None,
        ecdsa_builtin_applications: None,
        keccak_builtin_applications: None,
        memory_holes: None,
        pedersen_builtin_applications: None,
        poseidon_builtin_applications: None,
        range_check_builtin_applications: None,
        segment_arena_builtin: None,
        steps: 0,
        data_availability: starknet_types_rpc::DataAvailability { l1_data_gas: 0, l1_gas: 0 },
    }
}

// This sample chain is used for every rpcs that query info gotten from state updates.
pub struct SampleChainForStateUpdates {
    pub block_hashes: Vec<Felt>,
    pub state_roots: Vec<Felt>,
    pub class_hashes: Vec<Felt>,
    pub compiled_class_hashes: Vec<Felt>,
    pub contracts: Vec<Felt>,
    pub keys: Vec<Felt>,
    pub values: Vec<Felt>,
    pub state_diffs: Vec<StateDiff>,
}

#[fixture]
pub fn sample_chain_for_state_updates(
    rpc_test_setup: (Arc<MadaraBackend>, Starknet),
) -> (SampleChainForStateUpdates, Starknet) {
    let (backend, rpc) = rpc_test_setup;
    (make_sample_chain_for_state_updates(&backend), rpc)
}

/// State diff
pub fn make_sample_chain_for_state_updates(backend: &MadaraBackend) -> SampleChainForStateUpdates {
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
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header {
                            parent_block_hash: Felt::ZERO,
                            global_state_root: state_roots[0],
                            block_number: 0,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[0],
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[0].clone(),
                vec![],
                None,
                None,
                None,
            )
            .unwrap();

        // Block 1
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header {
                            parent_block_hash: block_hashes[0],
                            global_state_root: state_roots[1],
                            block_number: 1,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[1],
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[1].clone(),
                vec![],
                None,
                None,
                None,
            )
            .unwrap();

        // Block 2
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header {
                            parent_block_hash: block_hashes[1],
                            global_state_root: state_roots[2],
                            block_number: 2,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        block_hash: block_hashes[2],
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[2].clone(),
                vec![],
                None,
                None,
                None,
            )
            .unwrap();

        // Pending
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                        header: PendingHeader {
                            parent_block_hash: block_hashes[2],
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                state_diffs[3].clone(),
                vec![],
                None,
                None,
                None,
            )
            .unwrap();
    }

    SampleChainForStateUpdates {
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
