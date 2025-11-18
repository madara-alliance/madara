use httpmock::{Mock, MockServer};
use mc_gateway_client::GatewayProvider;
use mp_state_update::DeclaredClassItem;
use rstest::*;
use serde_json::{json, Value};
use starknet_core::types::Felt;
use std::sync::Arc;

pub struct GatewayMock {
    mock_server: MockServer,
}

#[fixture]
pub fn gateway_mock() -> GatewayMock {
    GatewayMock::new()
}

impl GatewayMock {
    pub fn new() -> Self {
        let mock_server = MockServer::start();
        Self { mock_server }
    }

    pub fn client(&self) -> Arc<GatewayProvider> {
        let address = self.mock_server.address();
        GatewayProvider::new(
            format!("http://{address}/gateway").parse().unwrap(),
            format!("http://{address}/feeder_gateway").parse().unwrap(),
        )
        .into()
    }

    pub fn mock_block_from_json(&self, block_number: u64, json: impl Into<String>) {
        self.mock_server.mock(|when, then| {
            when.method("GET").path_contains("get_state_update").query_param("blockNumber", block_number.to_string());
            then.status(200).header("content-type", "application/json").body(json.into());
        });
    }

    pub fn mock_class_from_json(&self, class_hash: impl Into<String>, json: impl Into<String>) {
        self.mock_server.mock(|when, then| {
            when.method("GET").path_contains("get_class_by_hash").query_param("classHash", class_hash);
            then.status(200).header("content-type", "application/json").body(json.into());
        });
    }

    pub fn mock_header_latest(&self, block_number: u64, hash: Felt) -> Mock<'_> {
        self.mock_server.mock(|when, then| {
            when.method("GET")
                .path_contains("get_block")
                .query_param("headerOnly", "true")
                .query_param("blockNumber", "latest");
            then.status(200).header("content-type", "application/json").json_body(json!({
                "block_number": block_number,
                "block_hash": format!("{hash:#x}"),
            }));
        })
    }

    pub fn mock_block(&self, block_number: u64, hash: Felt, parent_hash: Felt) {
        self.mock_block_with_declared_class(block_number, hash, parent_hash, None);
    }

    pub fn mock_block_with_declared_class(
        &self,
        block_number: u64,
        hash: Felt,
        parent_hash: Felt,
        declared_class: Option<DeclaredClassItem>,
    ) {
        let declared_classes = declared_class
            .map(|item| {
                json!({
                    "class_hash": item.class_hash,
                    "compiled_class_hash": item.compiled_class_hash
                })
            })
            .into_iter()
            .collect::<Vec<_>>();
        // block alpha-sepolia 172270
        self.mock_server.mock(|when, then| {
            when.method("GET").path_contains("get_state_update").query_param("blockNumber", block_number.to_string());
            then.status(200).header("content-type", "application/json").json_body(json!({
                "block": {
                    "block_hash": format!("{hash:#x}"),
                    "parent_block_hash": format!("{parent_hash:#x}"),
                    "block_number": block_number,
                    "state_root": "0x704b7fe29fa070cf3737173acd1d0790fe318f68cc07a49ddfa9c1cd94c804f",
                    "transaction_commitment": "0x4ff55c4b2d1784ba40da993ab03e0476c6466431681112000dca0eb6d7a29ae",
                    "event_commitment": "0x51f9c6962c8f93324ccf0b97a817f2e8ffbdd9c164d362bd1ea078c203677f4",
                    "receipt_commitment": "0x75b61baea9980d332a14fa78042e51b734f12bb69227ac2bd3acff9fbab0200",
                    "state_diff_commitment": "0x76e0f0a5468eaedb00ac7fec3307c0b4fad272bb6c2b775cc7a137a9bf052a",
                    "state_diff_length": 43,
                    "status": "ACCEPTED_ON_L1",
                    "l1_da_mode": "CALLDATA",
                    "l1_gas_price": {
                        "price_in_wei": "0x3bf1322e5",
                        "price_in_fri": "0x55dfe7f2de82"
                    },
                    "l1_data_gas_price": {
                        "price_in_wei": "0x3f9ffec0e7",
                        "price_in_fri": "0x5b269552db6fa"
                    },
                    "l2_gas_price": {
                        "price_in_wei": "0x1",
                        "price_in_fri": "0x1"
                    },
                    "transactions": [],
                    "timestamp": 1725974819,
                    "sequencer_address": "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                    "transaction_receipts": [],
                    "starknet_version": "0.13.2.1"
                },
                "state_update": {
                    "block_hash": "0x541112d5d5937a66ff09425a0256e53ac5c4f554be7e24917fc21a71aa3cf32",
                    "new_root": "0x704b7fe29fa070cf3737173acd1d0790fe318f68cc07a49ddfa9c1cd94c804f",
                    "old_root": "0x6152bda357cb522337756c71bcab298d88c5d829a479ad8247b82b969912713",
                    "state_diff": {
                        "storage_diffs": {
                            "0x36133c88c1954413150db74c26243e2af77170a4032934b275708d84ec5452f": [
                                {
                                    "key": "0x2306b6ab1b4c67429442feb1e6d238135a6cfcaa471a01b0e336f01b048e38",
                                    "value": "0x15"
                                }
                            ],
                            "0x36031daa264c24520b11d93af622c848b2499b66b41d611bac95e13cfca131a": [
                                {
                                    "key": "0x38502d057a7e5faeb88c2da2b38bed5cb3b54ba595bdaaffa08e00c1f23ff7",
                                    "value": "0x5f631d8000000000000000000000000066e04935"
                                },
                                {
                                    "key": "0xa1fb34bebf1a31f7f5655609661d0adf360ee017d59f5a79a888269f14610e",
                                    "value": "0x3686dbd65b000000000000000000000000066e04935"
                                },
                            ],
                            "0x1": [
                                {
                                    "key": "0x2a0e4",
                                    "value": "0x19fbf42069cb1630e398e3f09790f8f33761cfe5c1aa97fa303024c99765633"
                                }
                            ],
                            "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7": [
                                {
                                    "key": "0x7b950dd4a9e58a185d85ec8be94a1caf54c2f5330cbd28abad32674d27dac6",
                                    "value": "0x58031d4af919b5"
                                },
                                {
                                    "key": "0x1df152ff90ee62c3b2e6371df9bcfdaab763761afbb17039433e3a9ad76c34d",
                                    "value": "0x4b50c91700c6b9ad3"
                                },
                            ],
                            "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d": [
                                {
                                    "key": "0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a",
                                    "value": "0x1b77017df88b0858c9c29"
                                },
                                {
                                    "key": "0x5928e5598505749c60b49cc98e3acd5f3faa4a36910f50824395385b3c3a5c6",
                                    "value": "0xd655ecb9fc78a132c2"
                                }
                            ],
                            "0x786b58232e3830dfb3a4b3aee0cfebe12399b246e1a3befa1ea04ee50bda427": [
                                {
                                    "key": "0xcd66ed5b9515acc6c6fca5770b2535a5e78ba19758560c36ea2bed4cc2a404",
                                    "value": "0x1"
                                }
                            ]
                        },
                        "nonces": {
                            "0x5005f66205d5d1c08d23b2046a9fa44f27a21dc1ea205bd33c5d7c667df2d7b": "0x33f0",
                            "0x786b58232e3830dfb3a4b3aee0cfebe12399b246e1a3befa1ea04ee50bda427": "0x8",
                        },
                        "deployed_contracts": [],
                        "old_declared_contracts": [],
                        "declared_classes": declared_classes,
                        "replaced_classes": []
                    }
                }
            }));
        });
    }

    pub fn mock_block_pending(&self, block_number: u64) -> Mock<'_> {
        self.mock_block_pending_with_ts(block_number, 1725950824)
    }

    /// Ts is timestamp. We use that to differentiate pending blocks in the tests.
    pub fn mock_block_pending_with_ts(&self, block_number: u64, timestamp: usize) -> Mock<'_> {
        // block alpha-sepolia 171544
        self.mock_server.mock(|when, then| {
            when.method("GET")
                .path_contains("get_preconfirmed_block")
                .query_param("blockNumber", block_number.to_string());
            then.status(200).header("content-type", "application/json").json_body(json!({
                "status": "PRE_CONFIRMED",
                "starknet_version": "0.13.2.1",
                "l1_da_mode": "CALLDATA",
                "l1_gas_price": {
                    "price_in_wei": "0x274287586",
                    "price_in_fri": "0x363cc34e29f8"
                },
                "l1_data_gas_price": {
                    "price_in_wei": "0x2bc1e42413",
                    "price_in_fri": "0x3c735d85586c2"
                },
                "l2_gas_price": {
                    "price_in_wei": "0x1",
                    "price_in_fri": "0x1"
                },
                "l1_data_gas_price": {
                    "price_in_wei": "0x1",
                    "price_in_fri": "0x1"
                },
                "timestamp": timestamp,
                "sequencer_address": "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                "transactions": [
                    {
                        "transaction_hash": "0x6a5a493cf33919e58aa4c75777bffdef97c0e39cac968896d7bee8cc67905a1",
                        "version": "0x1",
                        "max_fee": "0x0",
                        "signature": [
                            "0x357dbb6c509a7d4b58f8ee7151236278b7959b39f7d05b8f7e2ef20593bdf7e",
                            "0x64d5f748eef19ca7f1c8cc533e5c9c85f80ef4f040c75da67bda82f5c58328d"
                        ],
                        "nonce": "0x1",
                        "sender_address": "0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8",
                        "calldata": [
                            "0x1",
                            "0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8",
                            "0x2730079d734ee55315f4f141eaed376bddd8c2133523d223a344c5604e0f7f8",
                            "0x0",
                            "0x5",
                            "0x5",
                            "0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3",
                            "0x322c2610264639f6b2cee681ac53fa65c37e187ea24292d1b21d859c55e1a78",
                            "0x1",
                            "0x0",
                            "0x1"
                        ],
                        "type": "INVOKE_FUNCTION"
                    },
                ],
                "transaction_receipts": [
                    {
                        "execution_status": "SUCCEEDED",
                        "transaction_index": 3,
                        "transaction_hash": "0x6a5a493cf33919e58aa4c75777bffdef97c0e39cac968896d7bee8cc67905a1",
                        "l2_to_l1_messages": [],
                        "events": [],
                        "execution_resources": {
                            "n_steps": 5617,
                            "builtin_instance_counter": {
                                "pedersen_builtin": 23,
                                "ecdsa_builtin": 1,
                                "range_check_builtin": 122
                            },
                            "n_memory_holes": 0
                        },
                        "actual_fee": "0x0"
                    },
                ],
                "transaction_state_diffs": [
                    {
                        "storage_diffs": {
                            "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d": [
                                {
                                    "key": "0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a",
                                    "value": "0x1b7622454b6cea6e76bb2"
                                },
                                {
                                    "key": "0x5928e5598505749c60b49cc98e3acd5f3faa4a36910f50824395385b3c3a5c6",
                                    "value": "0xdefb9937f1c6af5096"
                                }
                            ]
                        },
                        "nonces": {
                            "0x596d7421536f9d895015f207a6a349f54081634a25d4b403d3cd0363208ee1c": "0x2",
                            "0x2bb8a1f5a1241c1ebe8e10ff93b38ab097b1a20f77517997f8799829e096535": "0x18ab"
                        },
                        "deployed_contracts": [
                            {
                                "address": "0x596d7421536f9d895015f207a6a349f54081634a25d4b403d3cd0363208ee1c",
                                "class_hash": "0x36078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f"
                            },
                            {
                                "address": "0x7ab19cc28b12535df410edd1dbaad521ee83479b5936e00decdde5dd566c8b7",
                                "class_hash": "0x4ccf6144da19dc18c9f109a8a46e66ea2e08b2f22b03f895a715968d26622ea"
                            }
                        ],
                        "old_declared_contracts": [],
                        "declared_classes": [],
                        "replaced_classes": []
                    }
                ],
            }));
        })
    }

    pub fn mock_class(&self, contract_file: &[u8]) {
        let json: Value = serde_json::from_slice(contract_file).expect("Failed to parse JSON");

        // Convert ABI to string
        let abi_string = serde_json::to_string(&json["abi"]).expect("Failed to serialize ABI");

        // Transform the JSON to match the expected API response format
        let api_response = json!({
            "contract_class_version": json["contract_class_version"],
            "sierra_program": json["sierra_program"],
            "entry_points_by_type": json["entry_points_by_type"],
            "abi": abi_string,
        });

        self.mock_server.mock(|when, then| {
            when.method("GET").path_contains("get_class_by_hash");
            then.status(200).header("content-type", "application/json").json_body(api_response);
        });
    }

    #[allow(unused)]
    pub fn mock_signature(&self) {
        self.mock_server.mock(|when, then| {
            when.method("GET").path_contains("get_signature");
            then.status(200).header("content_type", "application/json").json_body(json!({
                    "block_hash": "0x541112d5d5937a66ff09425a0256e53ac5c4f554be7e24917fc21a71aa3cf32",
                    "signature": [
                        "0x315b1d77f8b1fc85657725639e88d4e1bfe846b4a866ddeb2e74cd91ccff9ca",
                        "0x3cbd913e55ca0c9ab107a5988dd4c54d56dd3700948a2b96c19d4728a5864de"
                    ]
            }));
        });
    }

    pub fn mock_block_pending_not_found(&self) -> Mock<'_> {
        self.mock_server.mock(|when, then| {
            when.method("GET").path_contains("get_state_update").query_param("blockNumber", "pending");
            then.status(400).header("content-type", "application/json").json_body(json!({
                "code": "StarknetErrorCode.BLOCK_NOT_FOUND",
                "message": "Block not found"
            }));
        })
    }
}
