//! These tests use the real FGW. They are very basic compared to the mock tests.

use super::*;
use rstest::{fixture, rstest};
use starknet_core::types::{DataAvailabilityResources, L1DataAvailabilityMode, ResourcePrice};
use starknet_providers::sequencer::models::{
    state_update::{StateDiff, StorageDiff},
    BlockStatus, BuiltinInstanceCounter, ConfirmedTransactionReceipt, Event, ExecutionResources,
    InvokeFunctionTransaction, TransactionExecutionStatus, TransactionType,
};

#[fixture]
fn sequencer_gateway_provider() -> SequencerGatewayProvider {
    SequencerGatewayProvider::starknet_alpha_mainnet()
}

#[rstest]
#[tokio::test]
async fn test_can_fetch_pending_block(sequencer_gateway_provider: SequencerGatewayProvider) {
    let block = fetch_pending_block_and_updates(&ChainId::Mainnet, &sequencer_gateway_provider).await.unwrap();
    // ignore as we can't check much here :/
    drop(block);
}

#[rstest]
#[tokio::test]
async fn test_can_convert_pending_block() {
    let felt = |s: &str| Felt::from_hex_unchecked(s);

    let block = (
        SequencerBlock {
            block_hash: None,
            block_number: None,
            parent_block_hash: felt("0x6c3b6c94d4add68c0309218708baa09463887f51863115346d380d67269a68a"),
            timestamp: 1726480207,
            sequencer_address: Some(felt("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8")),
            state_root: None,
            transaction_commitment: None,
            event_commitment: None,
            status: BlockStatus::Pending,
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            l1_gas_price: ResourcePrice { price_in_fri: felt("0x993f2ef1b9f1"), price_in_wei: felt("0x6894e3020") },
            l1_data_gas_price: ResourcePrice {
                price_in_fri: felt("0x375526f9a8f0f"),
                price_in_wei: felt("0x25c2d9f043"),
            },
            transactions: vec![
                TransactionType::InvokeFunction(InvokeFunctionTransaction {
                    sender_address: felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                    entry_point_selector: None,
                    calldata: vec![
                        felt("0x2"),
                        felt("0x0"),
                        felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                    ],
                    signature: vec![felt("0x3b81b87ad249504cd88f031c0b1c7a893eedb8bce50ca88c53a051014d0a25d")],
                    transaction_hash: felt("0x40a00bf7c2ccc154b4d18e63f072f69a0ce99c5249c2b7b62076dc017408858"),
                    max_fee: Some(felt("0x3f6c4a9a01d28")),
                    nonce: Some(felt("0x12")),
                    nonce_data_availability_mode: None,
                    fee_data_availability_mode: None,
                    resource_bounds: None,
                    tip: None,
                    paymaster_data: None,
                    account_deployment_data: None,
                    version: felt("0x1"),
                }),
                TransactionType::InvokeFunction(InvokeFunctionTransaction {
                    sender_address: felt("0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8"),
                    entry_point_selector: None,
                    calldata: vec![
                        felt("0x2"),
                        felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                    ],
                    signature: vec![
                        felt("0x16337397dd75f3d7f95bcca3509149b266c90ab5fdf87c7d950767b78fab231"),
                        felt("0x7bcc57dfc8159fa54de2d2e87ea6ecd4204e7cbc2173d0b2345d43c4028f944"),
                    ],
                    transaction_hash: felt("0x39441f846a99c8fec3cdb328bf00a9ee3d664f732f2c9649e324e61582762e8"),
                    max_fee: Some(felt("0x3f6c4a9191bc1")),
                    nonce: Some(felt("0x1b")),
                    nonce_data_availability_mode: None,
                    fee_data_availability_mode: None,
                    resource_bounds: None,
                    tip: None,
                    paymaster_data: None,
                    account_deployment_data: None,
                    version: felt("0x1"),
                }),
            ],
            transaction_receipts: vec![
                ConfirmedTransactionReceipt {
                    transaction_hash: felt("0x40a00bf7c2ccc154b4d18e63f072f69a0ce99c5249c2b7b62076dc017408858"),
                    transaction_index: 0,
                    execution_status: Some(TransactionExecutionStatus::Succeeded),
                    revert_error: None,
                    execution_resources: Some(ExecutionResources {
                        n_steps: 87793,
                        n_memory_holes: 0,
                        builtin_instance_counter: BuiltinInstanceCounter {
                            pedersen_builtin: Some(114),
                            range_check_builtin: Some(9393),
                            bitwise_builtin: Some(12),
                            output_builtin: None,
                            ecdsa_builtin: None,
                            ec_op_builtin: Some(6),
                            poseidon_builtin: None,
                            keccak_builtin: None,
                            segment_arena_builtin: None,
                        },
                        data_availability: Some(DataAvailabilityResources { l1_gas: 12838, l1_data_gas: 0 }),
                        total_gas_consumed: Some(DataAvailabilityResources { l1_gas: 13233, l1_data_gas: 0 }),
                    }),
                    l1_to_l2_consumed_message: None,
                    l2_to_l1_messages: vec![],
                    events: vec![
                        Event {
                            from_address: felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                            keys: vec![
                                felt("0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff"),
                                felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                                felt("0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"),
                            ],
                            data: vec![felt("0x7738106d5"), felt("0x0")],
                        },
                        Event {
                            from_address: felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                            keys: vec![
                                felt("0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff"),
                                felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                                felt("0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"),
                            ],
                            data: vec![felt("0x0"), felt("0x0")],
                        },
                        Event {
                            from_address: felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                            keys: vec![
                                felt("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                                felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                                felt("0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"),
                            ],
                            data: vec![felt("0x7738106d5"), felt("0x0")],
                        },
                    ],
                    actual_fee: felt("0x151df82a5a620"),
                },
                ConfirmedTransactionReceipt {
                    transaction_hash: felt("0x39441f846a99c8fec3cdb328bf00a9ee3d664f732f2c9649e324e61582762e8"),
                    transaction_index: 1,
                    execution_status: Some(TransactionExecutionStatus::Succeeded),
                    revert_error: None,
                    execution_resources: Some(ExecutionResources {
                        n_steps: 59639,
                        n_memory_holes: 0,
                        builtin_instance_counter: BuiltinInstanceCounter {
                            pedersen_builtin: Some(110),
                            range_check_builtin: Some(4129),
                            bitwise_builtin: Some(12),
                            output_builtin: None,
                            ecdsa_builtin: None,
                            ec_op_builtin: Some(6),
                            poseidon_builtin: None,
                            keccak_builtin: None,
                            segment_arena_builtin: None,
                        },
                        data_availability: Some(DataAvailabilityResources { l1_gas: 11736, l1_data_gas: 0 }),
                        total_gas_consumed: Some(DataAvailabilityResources { l1_gas: 11921, l1_data_gas: 0 }),
                    }),
                    l1_to_l2_consumed_message: None,
                    l2_to_l1_messages: vec![],
                    events: vec![Event {
                        from_address: felt("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
                        keys: vec![felt("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")],
                        data: vec![
                            felt("0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8"),
                            felt("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
                            felt("0x1305fd1ef0220"),
                            felt("0x0"),
                        ],
                    }],
                    actual_fee: felt("0x1305fd1ef0220"),
                },
            ],
            starknet_version: Some("0.13.2.1".into()),
        },
        SequencerStateUpdate {
            block_hash: None,
            new_root: None,
            old_root: felt("0x7b664bb7e0de32440cb925f3b0580a3f72d42442bd508fbe44f78956baec413"),
            state_diff: StateDiff {
                storage_diffs: [
                    (
                        felt("0x13e48fb52751595411acd0153a04be2f9472e23cc06eb52dd991145914234ec"),
                        vec![
                            StorageDiff {
                                key: felt("0x12f1312f6092a30a200f21c6e298f5a94b3522583fe40c98c035b7fd8049b0"),
                                value: felt("0x25b0807ac00"),
                            },
                            StorageDiff {
                                key: felt("0x1d411efc1b91ba8ec75807861b5ba3fc72f0df8d68a1155a6b4c6717865364"),
                                value: felt("0x3dc5cb078c05c00"),
                            },
                        ],
                    ),
                    (
                        felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                        vec![StorageDiff {
                            key: felt("0xbc72301fcd3b05852cc058ab4cae654f59901211906ce8eaab14eff5546aae"),
                            value: felt("0x0"),
                        }],
                    ),
                ]
                .into(),
                deployed_contracts: vec![],
                old_declared_contracts: vec![],
                declared_classes: vec![],
                replaced_classes: vec![],
                nonces: [
                    (felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"), felt("0x13")),
                    (felt("0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8"), felt("0x1c")),
                    (felt("0x2bb8a1f5a1241c1ebe8e10ff93b38ab097b1a20f77517997f8799829e096535"), felt("0x3af1")),
                ]
                .into(),
            },
        },
        vec![],
    );
    let converted = convert_sequencer_pending_block(block.0, block.1, block.2).unwrap();
    assert_eq!(
        converted,
        serde_json::from_value(serde_json::json!({
            "header": {
            "parent_block_hash": "0x6c3b6c94d4add68c0309218708baa09463887f51863115346d380d67269a68a",
            "sequencer_address": "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
            "block_timestamp": 1726480207,
            "protocol_version": [
                0,
                13,
                2,
                1
            ],
            "l1_gas_price": {
                "eth_l1_gas_price": 28073406496u64,
                "strk_l1_gas_price": 168496649583089u64,
                "eth_l1_data_gas_price": 162182852675u64,
                "strk_l1_data_gas_price": 973421850300175u64
            },
            "l1_da_mode": "Calldata"
            },
            "state_diff": {
            "storage_diffs": [
                {
                "address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                "storage_entries": [
                    {
                    "key": "0xbc72301fcd3b05852cc058ab4cae654f59901211906ce8eaab14eff5546aae",
                    "value": "0x0"
                    }
                ]
                },
                {
                "address": "0x13e48fb52751595411acd0153a04be2f9472e23cc06eb52dd991145914234ec",
                "storage_entries": [
                    {
                    "key": "0x12f1312f6092a30a200f21c6e298f5a94b3522583fe40c98c035b7fd8049b0",
                    "value": "0x25b0807ac00"
                    },
                    {
                    "key": "0x1d411efc1b91ba8ec75807861b5ba3fc72f0df8d68a1155a6b4c6717865364",
                    "value": "0x3dc5cb078c05c00"
                    }
                ]
                }
            ],
            "deprecated_declared_classes": [],
            "declared_classes": [],
            "deployed_contracts": [],
            "replaced_classes": [],
            "nonces": [
                {
                "contract_address": "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                "nonce": "0x13"
                },
                {
                "contract_address": "0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8",
                "nonce": "0x1c"
                },
                {
                "contract_address": "0x2bb8a1f5a1241c1ebe8e10ff93b38ab097b1a20f77517997f8799829e096535",
                "nonce": "0x3af1"
                }
            ]
            },
            "transactions": [
            {
                "Invoke": {
                "V1": {
                    "sender_address": "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                    "calldata": [
                    "0x2",
                    "0x0",
                    "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"
                    ],
                    "max_fee": "0x3f6c4a9a01d28",
                    "signature": [
                    "0x3b81b87ad249504cd88f031c0b1c7a893eedb8bce50ca88c53a051014d0a25d"
                    ],
                    "nonce": "0x12"
                }
                }
            },
            {
                "Invoke": {
                "V1": {
                    "sender_address": "0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8",
                    "calldata": [
                    "0x2",
                    "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"
                    ],
                    "max_fee": "0x3f6c4a9191bc1",
                    "signature": [
                    "0x16337397dd75f3d7f95bcca3509149b266c90ab5fdf87c7d950767b78fab231",
                    "0x7bcc57dfc8159fa54de2d2e87ea6ecd4204e7cbc2173d0b2345d43c4028f944"
                    ],
                    "nonce": "0x1b"
                }
                }
            }
            ],
            "receipts": [
            {
                "Invoke": {
                "transaction_hash": "0x40a00bf7c2ccc154b4d18e63f072f69a0ce99c5249c2b7b62076dc017408858",
                "actual_fee": {
                    "amount": "0x151df82a5a620",
                    "unit": "Wei"
                },
                "messages_sent": [],
                "events": [
                    {
                    "from_address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                    "keys": [
                        "0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff",
                        "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                        "0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"
                    ],
                    "data": [
                        "0x7738106d5",
                        "0x0"
                    ]
                    },
                    {
                    "from_address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                    "keys": [
                        "0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff",
                        "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                        "0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"
                    ],
                    "data": [
                        "0x0",
                        "0x0"
                    ]
                    },
                    {
                    "from_address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                    "keys": [
                        "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9",
                        "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                        "0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"
                    ],
                    "data": [
                        "0x7738106d5",
                        "0x0"
                    ]
                    }
                ],
                "execution_resources": {
                    "steps": 87793,
                    "memory_holes": 0,
                    "range_check_builtin_applications": 9393,
                    "pedersen_builtin_applications": 114,
                    "poseidon_builtin_applications": null,
                    "ec_op_builtin_applications": 6,
                    "ecdsa_builtin_applications": null,
                    "bitwise_builtin_applications": 12,
                    "keccak_builtin_applications": null,
                    "segment_arena_builtin": null,
                    "data_availability": {
                    "l1_gas": 12838,
                    "l1_data_gas": 0
                    },
                    "total_gas_consumed": {
                    "l1_gas": 13233,
                    "l1_data_gas": 0
                    }
                },
                "execution_result": "Succeeded"
                }
            },
            {
                "Invoke": {
                "transaction_hash": "0x39441f846a99c8fec3cdb328bf00a9ee3d664f732f2c9649e324e61582762e8",
                "actual_fee": {
                    "amount": "0x1305fd1ef0220",
                    "unit": "Wei"
                },
                "messages_sent": [],
                "events": [
                    {
                    "from_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                    "keys": [
                        "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                    ],
                    "data": [
                        "0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8",
                        "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                        "0x1305fd1ef0220",
                        "0x0"
                    ]
                    }
                ],
                "execution_resources": {
                    "steps": 59639,
                    "memory_holes": 0,
                    "range_check_builtin_applications": 4129,
                    "pedersen_builtin_applications": 110,
                    "poseidon_builtin_applications": null,
                    "ec_op_builtin_applications": 6,
                    "ecdsa_builtin_applications": null,
                    "bitwise_builtin_applications": 12,
                    "keccak_builtin_applications": null,
                    "segment_arena_builtin": null,
                    "data_availability": {
                    "l1_gas": 11736,
                    "l1_data_gas": 0
                    },
                    "total_gas_consumed": {
                    "l1_gas": 11921,
                    "l1_data_gas": 0
                    }
                },
                "execution_result": "Succeeded"
                }
            }
            ],
            "declared_classes": []
        }))
        .unwrap()
    )
}

// TODO: I'd like to have more tests for more starknt versions, but i don't want to commit multiple megabytes into the repository. These tests should be made with the mocked FGW.
#[rstest]
#[case(0)]
#[case(724_130)]
#[tokio::test]
async fn test_can_fetch_and_convert_block(sequencer_gateway_provider: SequencerGatewayProvider, #[case] block_n: u64) {
    let block = fetch_block_and_updates(&ChainId::Mainnet, block_n, &sequencer_gateway_provider).await.unwrap();
    let path = &format!("test-data/block_{block_n}.json");
    // serde_json::to_writer(std::fs::File::create(path).unwrap(), &block).unwrap();
    let expected = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(block, expected)
}
