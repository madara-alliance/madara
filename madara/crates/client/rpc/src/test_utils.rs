use crate::Starknet;
use jsonrpsee::core::async_trait;
use mc_db::{
    preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction},
    MadaraBackend,
};
use mc_submit_tx::{SubmitTransaction, SubmitTransactionError};
use mp_block::{
    header::{BlockTimestamp, GasPrices, PreconfirmedHeader},
    FullBlockWithoutCommitments, TransactionWithReceipt,
};
use mp_chain_config::ChainConfig;
use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
use mp_class::{
    CompiledSierra, ConvertedClass, EntryPointsByType, FlattenedSierraClass, SierraClassInfo, SierraConvertedClass,
};
use mp_receipt::{
    ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
};
#[cfg(test)]
use mp_rpc::v0_9_0::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_rpc::{admin::BroadcastedDeclareTxnV0, v0_9_0::ExecutionStatus};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use mp_transactions::{validated::TxTimestamp, InvokeTransaction, InvokeTransactionV0, Transaction};
use mp_utils::service::ServiceContext;
use rstest::fixture;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

#[cfg(test)]
pub struct TestTransactionProvider;

#[cfg(test)]
#[async_trait]
impl SubmitTransaction for TestTransactionProvider {
    async fn submit_declare_v0_transaction(
        &self,
        _declare_v0_transaction: BroadcastedDeclareTxnV0,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        unimplemented!()
    }
    async fn submit_declare_transaction(
        &self,
        _declare_transaction: BroadcastedDeclareTxn,
    ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
        unimplemented!()
    }
    async fn submit_deploy_account_transaction(
        &self,
        _deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
        unimplemented!()
    }
    async fn submit_invoke_transaction(
        &self,
        _invoke_transaction: BroadcastedInvokeTxn,
    ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
        unimplemented!()
    }
    async fn received_transaction(&self, _hash: mp_convert::Felt) -> Option<bool> {
        unimplemented!()
    }
    async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>> {
        unimplemented!()
    }
}

#[fixture]
pub fn rpc_test_setup() -> (Arc<MadaraBackend>, Starknet) {
    let chain_config = Arc::new(ChainConfig::madara_test());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let mut rpc = Starknet::new(
        backend.clone(),
        Arc::new(TestTransactionProvider),
        Default::default(),
        None,
        ServiceContext::new_for_testing(),
    );
    // Enable showing preconfirmed transactions in v0.7/v0.8 pending blocks for tests
    rpc.pre_v0_9_preconfirmed_as_pending = true;
    (backend, rpc)
}

// This sample chain is only used to test get tx / get block rpcs.
pub struct SampleChainForBlockGetters {
    pub block_hashes: Vec<Felt>,
    pub tx_hashes: Vec<Felt>,
    pub expected_txs_v0_7: Vec<mp_rpc::v0_7_1::TxnWithHash>,
    pub expected_txs_v0_8: Vec<mp_rpc::v0_8_1::TxnWithHash>,
    pub expected_receipts_v0_7: Vec<mp_rpc::v0_7_1::TxnReceipt>,
    pub expected_receipts_v0_8: Vec<mp_rpc::v0_8_1::TxnReceipt>,
    pub expected_receipts_v0_9: Vec<mp_rpc::v0_9_0::TxnReceipt>,
}

#[fixture]
pub fn sample_chain_for_block_getters(
    rpc_test_setup: (Arc<MadaraBackend>, Starknet),
) -> (SampleChainForBlockGetters, Starknet) {
    let (backend, rpc) = rpc_test_setup;
    (make_sample_chain_for_block_getters(&backend), rpc)
}

/// Transactions and blocks testing, no state diff, no converted class
pub fn make_sample_chain_for_block_getters(backend: &Arc<MadaraBackend>) -> SampleChainForBlockGetters {
    let mut block_hashes = vec![];
    let tx_hashes = vec![
        Felt::from_hex_unchecked("0x8888888"),
        Felt::from_hex_unchecked("0xdd848484"),
        Felt::from_hex_unchecked("0xdd84848407"),
        Felt::from_hex_unchecked("0xdd84847784"),
    ];
    let expected_txs_v0_7 = {
        use mp_rpc::v0_7_1::{InvokeTxn, InvokeTxnV0, Txn, TxnWithHash};
        vec![
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0x12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x4343"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828")].into(),
                })),
                transaction_hash: tx_hashes[0],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                })),
                transaction_hash: tx_hashes[1],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828eeb")].into(),
                })),
                transaction_hash: tx_hashes[2],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                })),
                transaction_hash: tx_hashes[3],
            },
        ]
    };
    let expected_txs_v0_8 = {
        use mp_rpc::v0_8_1::{InvokeTxn, InvokeTxnV0, Txn, TxnWithHash};
        vec![
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0x12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x4343"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828")].into(),
                })),
                transaction_hash: tx_hashes[0],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                })),
                transaction_hash: tx_hashes[1],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828eeb")].into(),
                })),
                transaction_hash: tx_hashes[2],
            },
            TxnWithHash {
                transaction: Txn::Invoke(InvokeTxn::V0(InvokeTxnV0 {
                    max_fee: Felt::from_hex_unchecked("0xb12"),
                    signature: vec![].into(),
                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                    calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                })),
                transaction_hash: tx_hashes[3],
            },
        ]
    };
    let expected_receipts_v0_7 = {
        use mp_rpc::v0_7_1::{
            CommonReceiptProperties, FeePayment, InvokeTxnReceipt, PriceUnit, TxnFinalityStatus, TxnReceipt,
        };
        vec![
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_7(),
                    finality_status: TxnFinalityStatus::L1,
                    execution_status: mp_rpc::v0_7_1::ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_7(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: mp_rpc::v0_7_1::ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94dd"), unit: PriceUnit::Fri },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_7(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: mp_rpc::v0_7_1::ExecutionStatus::Reverted("too bad".into()),
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_7(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: mp_rpc::v0_7_1::ExecutionStatus::Successful,
                },
            }),
        ]
    };
    let expected_receipts_v0_8 = {
        use mp_rpc::v0_8_1::{
            CommonReceiptProperties, FeePayment, InvokeTxnReceipt, PriceUnit, TxnFinalityStatus, TxnReceipt,
        };
        vec![
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L1,
                    execution_status: mp_rpc::v0_8_1::ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: mp_rpc::v0_8_1::ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94dd"), unit: PriceUnit::Fri },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: mp_rpc::v0_8_1::ExecutionStatus::Reverted("too bad".into()),
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                    actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: mp_rpc::v0_8_1::ExecutionStatus::Successful,
                },
            }),
        ]
    };
    let expected_receipts_v0_9 = {
        use mp_rpc::v0_9_0::{
            CommonReceiptProperties, FeePayment, InvokeTxnReceipt, PriceUnit, PriceUnitFri, PriceUnitWei,
            TxnFinalityStatus, TxnReceipt,
        };
        vec![
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0x8888888"),
                    actual_fee: FeePayment {
                        amount: Felt::from_hex_unchecked("0x9"),
                        unit: PriceUnit::Wei(PriceUnitWei::Wei),
                    },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L1,
                    execution_status: ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd848484"),
                    actual_fee: FeePayment {
                        amount: Felt::from_hex_unchecked("0x94"),
                        unit: PriceUnit::Wei(PriceUnitWei::Wei),
                    },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: ExecutionStatus::Successful,
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84848407"),
                    actual_fee: FeePayment {
                        amount: Felt::from_hex_unchecked("0x94dd"),
                        unit: PriceUnit::Fri(PriceUnitFri::Fri),
                    },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::L2,
                    execution_status: ExecutionStatus::Reverted("too bad".into()),
                },
            }),
            TxnReceipt::Invoke(InvokeTxnReceipt {
                common_receipt_properties: CommonReceiptProperties {
                    transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                    actual_fee: FeePayment {
                        amount: Felt::from_hex_unchecked("0x94"),
                        unit: PriceUnit::Wei(PriceUnitWei::Wei),
                    },
                    messages_sent: vec![],
                    events: vec![],
                    execution_resources: defaut_execution_resources_v0_8(),
                    finality_status: TxnFinalityStatus::PreConfirmed,
                    execution_status: ExecutionStatus::Successful,
                },
            }),
        ]
    };

    {
        // Block 0
        block_hashes.push(
            backend
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
                            l1_da_mode: L1DataAvailabilityMode::Blob,
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
                                actual_fee: FeePayment {
                                    amount: Felt::from_hex_unchecked("0x9"),
                                    unit: PriceUnit::Wei,
                                },
                                messages_sent: vec![],
                                events: vec![],
                                execution_resources: ExecutionResources::default(),
                                execution_result: ExecutionResult::Succeeded,
                            }),
                        }],
                        events: vec![],
                    },
                    &[],
                    false,
                )
                .unwrap()
                .block_hash,
        );

        // Block 1
        block_hashes.push(
            backend
                .write_access()
                .add_full_block_with_classes(
                    &FullBlockWithoutCommitments {
                        header: PreconfirmedHeader {
                            block_number: 1,
                            l1_da_mode: L1DataAvailabilityMode::Calldata,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        state_diff: Default::default(),
                        transactions: Default::default(),
                        events: Default::default(),
                    },
                    &[],
                    false,
                )
                .unwrap()
                .block_hash,
        );

        // Block 2
        block_hashes.push(
            backend
                .write_access()
                .add_full_block_with_classes(
                    &FullBlockWithoutCommitments {
                        header: PreconfirmedHeader {
                            block_number: 2,
                            l1_da_mode: L1DataAvailabilityMode::Blob,
                            protocol_version: StarknetVersion::V0_13_2,
                            ..Default::default()
                        },
                        state_diff: Default::default(),
                        transactions: vec![
                            TransactionWithReceipt {
                                transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                                    max_fee: Felt::from_hex_unchecked("0xb12"),
                                    signature: vec![].into(),
                                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                                    entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                                    calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                                })),
                                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
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
                            },
                            TransactionWithReceipt {
                                transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                                    max_fee: Felt::from_hex_unchecked("0xb12"),
                                    signature: vec![].into(),
                                    contract_address: Felt::from_hex_unchecked("0x434b3"),
                                    entry_point_selector: Felt::from_hex_unchecked("0x1212223"),
                                    calldata: vec![Felt::from_hex_unchecked("0x2828eeb")].into(),
                                })),
                                receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
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
                            },
                        ],
                        events: vec![],
                    },
                    &[],
                    true,
                )
                .unwrap()
                .block_hash,
        );

        // Pending
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    protocol_version: StarknetVersion::V0_13_2,
                    l1_da_mode: L1DataAvailabilityMode::Blob,
                    block_number: 3,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt {
                        transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                            max_fee: Felt::from_hex_unchecked("0xb12"),
                            signature: vec![].into(),
                            contract_address: Felt::from_hex_unchecked("0x434b3"),
                            entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                            calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                        })),
                        receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                            transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                            actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                            messages_sent: vec![],
                            events: vec![],
                            execution_resources: ExecutionResources::default(),
                            execution_result: ExecutionResult::Succeeded,
                        }),
                    },
                    state_diff: Default::default(),
                    declared_class: None,
                    arrived_at: Default::default(),
                    paid_fee_on_l1: None,
                }],
                [],
            ))
            .unwrap();
    }

    backend.set_latest_l1_confirmed(Some(0)).unwrap();

    SampleChainForBlockGetters {
        block_hashes,
        tx_hashes,
        expected_txs_v0_7,
        expected_txs_v0_8,
        expected_receipts_v0_7,
        expected_receipts_v0_8,
        expected_receipts_v0_9,
    }
}

fn defaut_execution_resources_v0_7() -> mp_rpc::v0_7_1::ExecutionResources {
    mp_rpc::v0_7_1::ExecutionResources {
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
        data_availability: mp_rpc::v0_7_1::DataAvailability { l1_data_gas: 0, l1_gas: 0 },
    }
}

fn defaut_execution_resources_v0_8() -> mp_rpc::v0_8_1::ExecutionResources {
    mp_rpc::v0_8_1::ExecutionResources { l1_gas: 0, l2_gas: 0, l1_data_gas: 0 }
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
pub fn make_sample_chain_for_state_updates(backend: &Arc<MadaraBackend>) -> SampleChainForStateUpdates {
    let mut block_hashes = vec![];
    let mut state_roots = vec![];
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
            old_declared_contracts: vec![],
            declared_classes: vec![
                DeclaredClassItem { class_hash: class_hashes[0], compiled_class_hash: compiled_class_hashes[0] },
                DeclaredClassItem { class_hash: class_hashes[1], compiled_class_hash: compiled_class_hashes[1] },
            ],
            deployed_contracts: vec![DeployedContractItem { address: contracts[0], class_hash: class_hashes[0] }],
            replaced_classes: vec![],
            nonces: vec![],
            migrated_compiled_classes: vec![],
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
            old_declared_contracts: vec![],
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
            migrated_compiled_classes: vec![],
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
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
            migrated_compiled_classes: vec![],
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
            old_declared_contracts: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![ReplacedClassItem { contract_address: contracts[0], class_hash: class_hashes[2] }],
            nonces: vec![
                NonceUpdate { contract_address: contracts[0], nonce: 3.into() },
                NonceUpdate { contract_address: contracts[1], nonce: 2.into() },
            ],
            migrated_compiled_classes: vec![],
        },
    ];

    // Block 0
    let res = backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader {
                    block_number: 0,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                state_diff: state_diffs[0].clone(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            true,
        )
        .unwrap();
    block_hashes.push(res.block_hash);
    state_roots.push(res.new_state_root);

    // Block 1
    let res = backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader {
                    block_number: 1,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                state_diff: state_diffs[1].clone(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            true,
        )
        .unwrap();
    block_hashes.push(res.block_hash);
    state_roots.push(res.new_state_root);

    // Block 2
    let res = backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader {
                    block_number: 2,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                state_diff: state_diffs[2].clone(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            true,
        )
        .unwrap();
    block_hashes.push(res.block_hash);
    state_roots.push(res.new_state_root);

    // Pending
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { protocol_version: StarknetVersion::V0_13_2, block_number: 3, ..Default::default() },
            vec![PreconfirmedExecutedTransaction {
                transaction: TransactionWithReceipt {
                    transaction: Transaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                        max_fee: Felt::from_hex_unchecked("0xb12"),
                        signature: vec![].into(),
                        contract_address: Felt::from_hex_unchecked("0x434b3"),
                        entry_point_selector: Felt::from_hex_unchecked("0x12123"),
                        calldata: vec![Felt::from_hex_unchecked("0x2828b")].into(),
                    })),
                    receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                        transaction_hash: Felt::from_hex_unchecked("0xdd84847784"),
                        actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x94"), unit: PriceUnit::Wei },
                        messages_sent: vec![],
                        events: vec![],
                        execution_resources: ExecutionResources::default(),
                        execution_result: ExecutionResult::Succeeded,
                    }),
                },
                state_diff: state_diffs[3].clone().into(),
                declared_class: Some(ConvertedClass::Sierra(SierraConvertedClass {
                    class_hash: class_hashes[2],
                    info: SierraClassInfo {
                        contract_class: FlattenedSierraClass {
                            sierra_program: vec![],
                            contract_class_version: Default::default(),
                            entry_points_by_type: EntryPointsByType {
                                constructor: vec![],
                                external: vec![],
                                l1_handler: vec![],
                            },
                            abi: Default::default(),
                        }
                        .into(),
                        compiled_class_hash: Some(compiled_class_hashes[2]),
                        compiled_class_hash_v2: None,
                    },
                    compiled: CompiledSierra(Default::default()).into(),
                })),
                arrived_at: TxTimestamp::default(),
                paid_fee_on_l1: None,
            }],
            [],
        ))
        .unwrap();

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
