//! Tests for v0.10.1 RPC types
//!
//! These tests follow TDD principles - written before the full implementation.

use super::*;
use starknet_types_core::felt::Felt;

// ============================================================================
// Address Filter Tests (inspired by Pathfinder PR #3180)
// ============================================================================

#[test]
fn test_parsing_multiple_addresses() {
    // Test deserialization of multiple addresses in filter
    let filter_json = r#"{
        "address": ["0x10", "0x20"],
        "chunk_size": 100
    }"#;

    let filter: EventFilterWithPageRequest = serde_json::from_str(filter_json).unwrap();
    match filter.address {
        Some(AddressFilter::Multiple(addrs)) => {
            assert_eq!(addrs.len(), 2);
            assert!(addrs.contains(&Felt::from_hex("0x10").unwrap()));
            assert!(addrs.contains(&Felt::from_hex("0x20").unwrap()));
        }
        _ => panic!("Expected multiple addresses"),
    }
}

#[test]
fn test_parsing_single_address_backward_compat() {
    // Test backward compatibility with single address
    let filter_json = r#"{
        "address": "0x10",
        "chunk_size": 100
    }"#;

    let filter: EventFilterWithPageRequest = serde_json::from_str(filter_json).unwrap();
    match filter.address {
        Some(AddressFilter::Single(addr)) => {
            assert_eq!(addr, Felt::from_hex("0x10").unwrap());
        }
        _ => panic!("Expected single address"),
    }
}

#[test]
fn test_empty_address_array() {
    // Empty array should deserialize properly
    let filter_json = r#"{
        "address": [],
        "chunk_size": 100
    }"#;

    let filter: EventFilterWithPageRequest = serde_json::from_str(filter_json).unwrap();
    match filter.address {
        Some(AddressFilter::Multiple(addrs)) => {
            assert!(addrs.is_empty());
        }
        _ => panic!("Expected empty address array"),
    }
}

#[test]
fn test_no_address_filter() {
    // No address field should result in None
    let filter_json = r#"{
        "chunk_size": 100
    }"#;

    let filter: EventFilterWithPageRequest = serde_json::from_str(filter_json).unwrap();
    assert!(filter.address.is_none());
}

#[test]
fn test_address_filter_to_set() {
    // Test conversion to HashSet for single address
    let single = AddressFilter::Single(Felt::from_hex("0x10").unwrap());
    let set = single.to_set().unwrap();
    assert_eq!(set.len(), 1);
    assert!(set.contains(&Felt::from_hex("0x10").unwrap()));

    // Test conversion for multiple addresses
    let multiple = AddressFilter::Multiple(vec![Felt::from_hex("0x10").unwrap(), Felt::from_hex("0x20").unwrap()]);
    let set = multiple.to_set().unwrap();
    assert_eq!(set.len(), 2);

    // Test empty array returns None (match all)
    let empty = AddressFilter::Multiple(vec![]);
    assert!(empty.to_set().is_none());
}

#[test]
fn test_address_filter_matches() {
    let addr1 = Felt::from_hex("0x10").unwrap();
    let addr2 = Felt::from_hex("0x20").unwrap();
    let addr3 = Felt::from_hex("0x30").unwrap();

    // Single address filter
    let single = AddressFilter::Single(addr1);
    assert!(single.matches(&addr1));
    assert!(!single.matches(&addr2));

    // Multiple address filter
    let multiple = AddressFilter::Multiple(vec![addr1, addr2]);
    assert!(multiple.matches(&addr1));
    assert!(multiple.matches(&addr2));
    assert!(!multiple.matches(&addr3));

    // Empty array matches all
    let empty = AddressFilter::Multiple(vec![]);
    assert!(empty.matches(&addr1));
    assert!(empty.matches(&addr2));
    assert!(empty.matches(&addr3));
}

// ============================================================================
// Simulation Flag Tests
// ============================================================================

#[test]
fn test_simulation_flag_skip_fee_charge() {
    let flags: Vec<SimulationFlag> = serde_json::from_str(r#"["SKIP_FEE_CHARGE"]"#).unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0], SimulationFlag::SkipFeeCharge);
}

#[test]
fn test_simulation_flag_skip_validate() {
    let flags: Vec<SimulationFlag> = serde_json::from_str(r#"["SKIP_VALIDATE"]"#).unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0], SimulationFlag::SkipValidate);
}

#[test]
fn test_simulation_flag_return_initial_reads() {
    // Test that RETURN_INITIAL_READS flag is parsed correctly (NEW in v0.10.1)
    let flags: Vec<SimulationFlag> = serde_json::from_str(r#"["SKIP_VALIDATE", "RETURN_INITIAL_READS"]"#).unwrap();
    assert_eq!(flags.len(), 2);
    assert!(flags.contains(&SimulationFlag::SkipValidate));
    assert!(flags.contains(&SimulationFlag::ReturnInitialReads));
}

#[test]
fn test_simulation_flag_serialization() {
    let flags = vec![SimulationFlag::SkipFeeCharge, SimulationFlag::ReturnInitialReads];
    let json = serde_json::to_string(&flags).unwrap();
    assert!(json.contains("SKIP_FEE_CHARGE"));
    assert!(json.contains("RETURN_INITIAL_READS"));
}

// ============================================================================
// Trace Flag Tests
// ============================================================================

#[test]
fn test_trace_flag_return_initial_reads() {
    let flags: Vec<TraceFlag> = serde_json::from_str(r#"["RETURN_INITIAL_READS"]"#).unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0], TraceFlag::ReturnInitialReads);
}

// ============================================================================
// Initial Reads Tests
// ============================================================================

#[test]
fn test_initial_reads_serialization() {
    let reads = InitialReads {
        storage: vec![InitialStorageRead {
            contract_address: Felt::from_hex("0x1").unwrap(),
            key: Felt::from_hex("0x2").unwrap(),
            value: Felt::from_hex("0x3").unwrap(),
        }],
        nonces: vec![InitialNonceRead {
            contract_address: Felt::from_hex("0x1").unwrap(),
            nonce: Felt::from_hex("0x10").unwrap(),
        }],
        class_hashes: vec![InitialClassHashRead {
            contract_address: Felt::from_hex("0x1").unwrap(),
            class_hash: Felt::from_hex("0x100").unwrap(),
        }],
        declared_contracts: vec![InitialDeclaredContract {
            class_hash: Felt::from_hex("0x100").unwrap(),
            is_declared: true,
        }],
    };

    let json = serde_json::to_string(&reads).unwrap();
    assert!(json.contains("storage"));
    assert!(json.contains("nonces"));
    assert!(json.contains("class_hashes"));
    assert!(json.contains("declared_contracts"));

    // Verify round-trip
    let deserialized: InitialReads = serde_json::from_str(&json).unwrap();
    assert_eq!(reads, deserialized);
}

#[test]
fn test_initial_reads_default() {
    let reads = InitialReads::default();
    assert!(reads.storage.is_empty());
    assert!(reads.nonces.is_empty());
    assert!(reads.class_hashes.is_empty());
    assert!(reads.declared_contracts.is_empty());
}

#[test]
fn test_initial_reads_deserialization_with_missing_fields() {
    // Test that missing fields default to empty arrays
    let json = r#"{"storage": []}"#;
    let reads: InitialReads = serde_json::from_str(json).unwrap();
    assert!(reads.storage.is_empty());
    assert!(reads.nonces.is_empty());
    assert!(reads.class_hashes.is_empty());
    assert!(reads.declared_contracts.is_empty());
}

// ============================================================================
// Response Flag Tests (from Pathfinder PR #3190)
// ============================================================================

#[test]
fn test_response_flags_include_proof_facts() {
    let flags: Vec<ResponseFlag> = serde_json::from_str(r#"["INCLUDE_PROOF_FACTS"]"#).unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0], ResponseFlag::IncludeProofFacts);
}

#[test]
fn test_response_flags_serialization() {
    let flags = vec![ResponseFlag::IncludeProofFacts];
    let json = serde_json::to_string(&flags).unwrap();
    assert!(json.contains("INCLUDE_PROOF_FACTS"));
}

// ============================================================================
// Subscription Tag Tests
// ============================================================================

#[test]
fn test_subscription_tag_include_proof_facts() {
    let tags: Vec<SubscriptionTag> = serde_json::from_str(r#"["INCLUDE_PROOF_FACTS"]"#).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0], SubscriptionTag::IncludeProofFacts);
}

// ============================================================================
// SimulateTransactionsResponse Tests
// ============================================================================

#[test]
fn test_simulate_transactions_response_without_initial_reads() {
    let response_json = r#"{
        "simulated_transactions": [
            {
                "fee_estimation": {
                    "overall_fee": "0x100",
                    "l1_gas_consumed": "0x10",
                    "l1_gas_price": "0x1",
                    "l1_data_gas_consumed": "0x5",
                    "l1_data_gas_price": "0x1",
                    "l2_gas_consumed": "0x20",
                    "l2_gas_price": "0x1",
                    "unit": "FRI"
                },
                "transaction_trace": {
                    "type": "INVOKE",
                    "execute_invocation": {
                        "revert_reason": "test"
                    },
                    "execution_resources": {
                        "l1_gas": "0x64",
                        "l1_data_gas": "0x32",
                        "l2_gas": "0xc8"
                    }
                }
            }
        ]
    }"#;

    let response: SimulateTransactionsResponse = serde_json::from_str(response_json).unwrap();
    assert!(response.initial_reads.is_none());
    assert_eq!(response.simulated_transactions.len(), 1);
}

#[test]
fn test_simulate_transactions_response_with_initial_reads() {
    let response_json = r#"{
        "simulated_transactions": [
            {
                "fee_estimation": {
                    "overall_fee": "0x100",
                    "l1_gas_consumed": "0x10",
                    "l1_gas_price": "0x1",
                    "l1_data_gas_consumed": "0x5",
                    "l1_data_gas_price": "0x1",
                    "l2_gas_consumed": "0x20",
                    "l2_gas_price": "0x1",
                    "unit": "FRI"
                },
                "transaction_trace": {
                    "type": "INVOKE",
                    "execute_invocation": {
                        "revert_reason": "test"
                    },
                    "execution_resources": {
                        "l1_gas": "0x64",
                        "l1_data_gas": "0x32",
                        "l2_gas": "0xc8"
                    }
                }
            }
        ],
        "initial_reads": {
            "storage": [{"contract_address": "0x1", "key": "0x2", "value": "0x3"}],
            "nonces": [],
            "class_hashes": [],
            "declared_contracts": []
        }
    }"#;

    let response: SimulateTransactionsResponse = serde_json::from_str(response_json).unwrap();
    let reads = response.initial_reads.expect("initial_reads should be present");
    assert_eq!(reads.storage.len(), 1);
}

#[test]
fn test_simulate_transactions_response_serialization_top_level_initial_reads() {
    let result_json = r#"{
        "fee_estimation": {
            "overall_fee": "0x100",
            "l1_gas_consumed": "0x10",
            "l1_gas_price": "0x1",
            "l1_data_gas_consumed": "0x5",
            "l1_data_gas_price": "0x1",
            "l2_gas_consumed": "0x20",
            "l2_gas_price": "0x1",
            "unit": "FRI"
        },
        "transaction_trace": {
            "type": "INVOKE",
            "execute_invocation": {
                "revert_reason": "test"
            },
            "execution_resources": {
                "l1_gas": "0x64",
                "l1_data_gas": "0x32",
                "l2_gas": "0xc8"
            }
        }
    }"#;

    let result: SimulateTransactionsResult = serde_json::from_str(result_json).unwrap();
    let response = SimulateTransactionsResponse {
        simulated_transactions: vec![result],
        initial_reads: Some(InitialReads {
            storage: vec![InitialStorageRead {
                contract_address: Felt::from_hex("0x1").unwrap(),
                key: Felt::from_hex("0x2").unwrap(),
                value: Felt::from_hex("0x3").unwrap(),
            }],
            nonces: vec![],
            class_hashes: vec![],
            declared_contracts: vec![],
        }),
    };

    let value = serde_json::to_value(&response).unwrap();
    assert!(value.get("initial_reads").is_some());

    let items = value
        .get("simulated_transactions")
        .and_then(|entry| entry.as_array())
        .expect("simulated_transactions should be an array");
    assert!(items[0].get("initial_reads").is_none());
}

// ============================================================================
// TraceBlockTransactionsResponse Tests
// ============================================================================

#[test]
fn test_trace_block_transactions_result_without_initial_reads() {
    let result_json = r#"{
        "trace_root": {
            "type": "INVOKE",
            "execute_invocation": {
                "revert_reason": "test"
            },
            "execution_resources": {
                "l1_gas": "0x64",
                "l1_data_gas": "0x32",
                "l2_gas": "0xc8"
            }
        },
        "transaction_hash": "0x123"
    }"#;

    let result: TraceBlockTransactionsResult = serde_json::from_str(result_json).unwrap();
    assert_eq!(result.transaction_hash, Felt::from_hex("0x123").unwrap());
}

#[test]
fn test_trace_block_transactions_response_without_initial_reads() {
    let response_json = r#"{
        "traces": [
            {
                "trace_root": {
                    "type": "INVOKE",
                    "execute_invocation": {
                        "revert_reason": "test"
                    },
                    "execution_resources": {
                        "l1_gas": "0x64",
                        "l1_data_gas": "0x32",
                        "l2_gas": "0xc8"
                    }
                },
                "transaction_hash": "0x123"
            }
        ]
    }"#;

    let response: TraceBlockTransactionsResponse = serde_json::from_str(response_json).unwrap();
    assert!(response.initial_reads.is_none());
    assert_eq!(response.traces.len(), 1);
}

#[test]
fn test_trace_block_transactions_response_with_initial_reads() {
    let response_json = r#"{
        "traces": [
            {
                "trace_root": {
                    "type": "INVOKE",
                    "execute_invocation": {
                        "revert_reason": "test"
                    },
                    "execution_resources": {
                        "l1_gas": "0x64",
                        "l1_data_gas": "0x32",
                        "l2_gas": "0xc8"
                    }
                },
                "transaction_hash": "0x123"
            }
        ],
        "initial_reads": {
            "storage": [{"contract_address": "0x1", "key": "0x2", "value": "0x3"}],
            "nonces": [],
            "class_hashes": [],
            "declared_contracts": []
        }
    }"#;

    let response: TraceBlockTransactionsResponse = serde_json::from_str(response_json).unwrap();
    let reads = response.initial_reads.expect("initial_reads should be present");
    assert_eq!(reads.storage.len(), 1);
}

// ============================================================================
// Backward Compatibility Tests
// ============================================================================

#[test]
fn test_event_filter_full_example() {
    // Full example with all fields
    let filter_json = r#"{
        "address": ["0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7", "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"],
        "from_block": {"block_number": 0},
        "to_block": {"block_number": 100},
        "keys": [["0x1", "0x2"], ["0x3"]],
        "chunk_size": 50,
        "continuation_token": "abc123"
    }"#;

    let filter: EventFilterWithPageRequest = serde_json::from_str(filter_json).unwrap();
    assert!(matches!(filter.address, Some(AddressFilter::Multiple(_))));
    assert_eq!(filter.chunk_size, 50);
    assert_eq!(filter.continuation_token, Some("abc123".to_string()));
}

#[test]
fn test_event_filter_serialization_roundtrip() {
    let filter = EventFilterWithPageRequest {
        address: Some(AddressFilter::Multiple(vec![Felt::from_hex("0x10").unwrap(), Felt::from_hex("0x20").unwrap()])),
        from_block: None,
        to_block: None,
        keys: Some(vec![vec![Felt::from_hex("0x1").unwrap()]]),
        chunk_size: 100,
        continuation_token: None,
    };

    let json = serde_json::to_string(&filter).unwrap();
    let deserialized: EventFilterWithPageRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(filter, deserialized);
}

// ============================================================================
// L1TxnHash Tests
// ============================================================================

#[test]
fn test_l1_txn_hash_deserialize_valid() {
    let hash: L1TxnHash = serde_json::from_str("\"0x1234\"").unwrap();
    assert_eq!(hash.as_str(), "0x1234");
}

#[test]
fn test_l1_txn_hash_deserialize_invalid_prefix() {
    let err = serde_json::from_str::<L1TxnHash>("\"1234\"").unwrap_err();
    assert!(err.to_string().contains("expected hex string starting with 0x"));
}

// ============================================================================
// proof_facts Backward Compatibility Tests
// ============================================================================
// These tests verify that old transactions without proof_facts can still be
// deserialized correctly (backward compatibility), and that new transactions
// with proof_facts work as expected.

#[test]
fn test_invoke_txn_v3_without_proof_facts_backward_compat() {
    // Old transaction format without proof_facts field
    // This simulates reading from a database with old transactions
    let txn_json = r#"{
        "sender_address": "0x1",
        "calldata": ["0x2", "0x3"],
        "signature": ["0x4"],
        "nonce": "0x5",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1"
    }"#;

    // Should deserialize successfully with proof_facts defaulting to None
    let txn: InvokeTxnV3 = serde_json::from_str(txn_json).unwrap();
    assert!(txn.proof_facts.is_none());
    assert_eq!(txn.inner.sender_address, Felt::from_hex("0x1").unwrap());
}

#[test]
fn test_invoke_txn_v3_with_proof_facts() {
    // New transaction format with proof_facts field
    let txn_json = r#"{
        "sender_address": "0x1",
        "calldata": ["0x2", "0x3"],
        "signature": ["0x4"],
        "nonce": "0x5",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "proof_facts": ["0x100", "0x200", "0x300"]
    }"#;

    let txn: InvokeTxnV3 = serde_json::from_str(txn_json).unwrap();
    assert!(txn.proof_facts.is_some());
    let proof_facts = txn.proof_facts.unwrap();
    assert_eq!(proof_facts.len(), 3);
    assert_eq!(proof_facts[0], Felt::from_hex("0x100").unwrap());
}

#[test]
fn test_invoke_txn_v3_with_empty_proof_facts() {
    // Transaction with empty proof_facts array
    let txn_json = r#"{
        "sender_address": "0x1",
        "calldata": ["0x2"],
        "signature": ["0x4"],
        "nonce": "0x5",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "proof_facts": []
    }"#;

    let txn: InvokeTxnV3 = serde_json::from_str(txn_json).unwrap();
    // Empty array should deserialize as Some([]) not None
    assert!(txn.proof_facts.is_some());
    assert!(txn.proof_facts.unwrap().is_empty());
}

#[test]
fn test_invoke_txn_v3_serialization_skips_none_proof_facts() {
    // When proof_facts is None, it should not appear in serialized output
    let txn = InvokeTxnV3 {
        inner: crate::v0_10_0::InvokeTxnV3 {
            sender_address: Felt::from_hex("0x1").unwrap(),
            calldata: vec![Felt::from_hex("0x2").unwrap()].into(),
            signature: vec![Felt::from_hex("0x3").unwrap()].into(),
            nonce: Felt::from_hex("0x4").unwrap(),
            resource_bounds: crate::v0_10_0::ResourceBoundsMapping {
                l1_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x10, max_price_per_unit: 0x1 },
                l2_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x20, max_price_per_unit: 0x2 },
                l1_data_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x30, max_price_per_unit: 0x3 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: crate::v0_10_0::DaMode::L1,
            fee_data_availability_mode: crate::v0_10_0::DaMode::L1,
        },
        proof_facts: None,
    };

    let json = serde_json::to_string(&txn).unwrap();
    // proof_facts should NOT appear in the output when it's None
    assert!(!json.contains("proof_facts"));
}

#[test]
fn test_invoke_txn_v3_serialization_includes_proof_facts_when_present() {
    let txn = InvokeTxnV3 {
        inner: crate::v0_10_0::InvokeTxnV3 {
            sender_address: Felt::from_hex("0x1").unwrap(),
            calldata: vec![Felt::from_hex("0x2").unwrap()].into(),
            signature: vec![Felt::from_hex("0x3").unwrap()].into(),
            nonce: Felt::from_hex("0x4").unwrap(),
            resource_bounds: crate::v0_10_0::ResourceBoundsMapping {
                l1_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x10, max_price_per_unit: 0x1 },
                l2_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x20, max_price_per_unit: 0x2 },
                l1_data_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x30, max_price_per_unit: 0x3 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: crate::v0_10_0::DaMode::L1,
            fee_data_availability_mode: crate::v0_10_0::DaMode::L1,
        },
        proof_facts: Some(vec![Felt::from_hex("0x100").unwrap()]),
    };

    let json = serde_json::to_string(&txn).unwrap();
    // proof_facts should appear in the output
    assert!(json.contains("proof_facts"));
    assert!(json.contains("0x100"));
}

// ============================================================================
// TxnWithProofFacts Tests (v0.10.1 specific types)
// ============================================================================

#[test]
fn test_txn_with_proof_facts_invoke_v3() {
    // Test deserialization of TxnWithProofFacts for INVOKE V3
    let txn_json = r#"{
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": "0x1",
        "calldata": ["0x2"],
        "signature": ["0x3"],
        "nonce": "0x4",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "proof_facts": ["0x100"]
    }"#;

    let txn: TxnWithProofFacts = serde_json::from_str(txn_json).unwrap();
    match txn {
        TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(invoke)) => {
            assert!(invoke.proof_facts.is_some());
            assert_eq!(invoke.proof_facts.unwrap()[0], Felt::from_hex("0x100").unwrap());
        }
        _ => panic!("Expected INVOKE V3 transaction"),
    }
}

#[test]
fn test_txn_with_proof_facts_invoke_v3_no_proof_facts() {
    // Old V3 transaction without proof_facts
    let txn_json = r#"{
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": "0x1",
        "calldata": ["0x2"],
        "signature": ["0x3"],
        "nonce": "0x4",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1"
    }"#;

    let txn: TxnWithProofFacts = serde_json::from_str(txn_json).unwrap();
    match txn {
        TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(invoke)) => {
            // proof_facts should default to None for old transactions
            assert!(invoke.proof_facts.is_none());
        }
        _ => panic!("Expected INVOKE V3 transaction"),
    }
}

#[test]
fn test_txn_with_hash_and_proof_facts() {
    let txn_json = r#"{
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": "0x1",
        "calldata": ["0x2"],
        "signature": ["0x3"],
        "nonce": "0x4",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "proof_facts": ["0x100"],
        "transaction_hash": "0xabc"
    }"#;

    let txn: TxnWithHashAndProofFacts = serde_json::from_str(txn_json).unwrap();
    assert_eq!(txn.transaction_hash, Felt::from_hex("0xabc").unwrap());
    match txn.transaction {
        TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(invoke)) => {
            assert!(invoke.proof_facts.is_some());
        }
        _ => panic!("Expected INVOKE V3 transaction"),
    }
}

// ============================================================================
// BroadcastedInvokeTxnV3 proof Tests
// ============================================================================

#[test]
fn test_broadcasted_invoke_txn_v3_with_proof() {
    // Test that proof field (array of integers) is handled correctly
    let txn_json = r#"{
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": "0x1",
        "calldata": ["0x2"],
        "signature": ["0x3"],
        "nonce": "0x4",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1",
        "proof": [1, 2, 3, 4, 5]
    }"#;

    let txn: BroadcastedInvokeTxnV3 = serde_json::from_str(txn_json).unwrap();
    assert!(txn.proof.is_some());
    let proof = txn.proof.unwrap();
    assert_eq!(proof.len(), 5);
    assert_eq!(proof[0], 1u64);
    assert_eq!(proof[4], 5u64);
}

#[test]
fn test_broadcasted_invoke_txn_v3_without_proof() {
    // Backward compatibility: no proof field
    let txn_json = r#"{
        "type": "INVOKE",
        "version": "0x3",
        "sender_address": "0x1",
        "calldata": ["0x2"],
        "signature": ["0x3"],
        "nonce": "0x4",
        "resource_bounds": {
            "l1_gas": {"max_amount": "0x10", "max_price_per_unit": "0x1"},
            "l2_gas": {"max_amount": "0x20", "max_price_per_unit": "0x2"},
            "l1_data_gas": {"max_amount": "0x30", "max_price_per_unit": "0x3"}
        },
        "tip": "0x0",
        "paymaster_data": [],
        "account_deployment_data": [],
        "nonce_data_availability_mode": "L1",
        "fee_data_availability_mode": "L1"
    }"#;

    let txn: BroadcastedInvokeTxnV3 = serde_json::from_str(txn_json).unwrap();
    assert!(txn.proof.is_none());
}

// ============================================================================
// Block Types with proof_facts Tests (v0.10.1 specific)
// ============================================================================
// These tests verify the new block types that include proof_facts support
// when INCLUDE_PROOF_FACTS response flag is set.

/// Helper function to create a test BlockHeader
fn create_test_block_header() -> crate::v0_10_0::BlockHeader {
    crate::v0_10_0::BlockHeader {
        block_hash: Felt::from_hex("0x123").unwrap(),
        block_number: 100,
        l1_da_mode: crate::v0_10_0::L1DaMode::Blob,
        l1_data_gas_price: crate::v0_10_0::ResourcePrice {
            price_in_fri: Felt::from_hex("0x1").unwrap(),
            price_in_wei: Felt::from_hex("0x1").unwrap(),
        },
        l1_gas_price: crate::v0_10_0::ResourcePrice {
            price_in_fri: Felt::from_hex("0x10").unwrap(),
            price_in_wei: Felt::from_hex("0x10").unwrap(),
        },
        l2_gas_price: crate::v0_10_0::ResourcePrice {
            price_in_fri: Felt::from_hex("0x5").unwrap(),
            price_in_wei: Felt::from_hex("0x5").unwrap(),
        },
        new_root: Felt::from_hex("0xabc").unwrap(),
        parent_hash: Felt::from_hex("0x122").unwrap(),
        sequencer_address: Felt::from_hex("0x1234").unwrap(),
        starknet_version: "0.13.4".to_string(),
        timestamp: 1700000000,
        event_commitment: Felt::from_hex("0xaaa1").unwrap(),
        transaction_commitment: Felt::from_hex("0xabc1").unwrap(),
        receipt_commitment: Felt::from_hex("0xabc2").unwrap(),
        state_diff_commitment: Felt::from_hex("0xabc3").unwrap(),
        // v0.10.0 new fields
        event_count: 10,
        transaction_count: 5,
        state_diff_length: 100,
    }
}

/// Helper function to create a test PreConfirmedBlockHeader
fn create_test_preconfirmed_block_header() -> crate::v0_9_0::PreConfirmedBlockHeader {
    crate::v0_9_0::PreConfirmedBlockHeader {
        l1_da_mode: crate::v0_10_0::L1DaMode::Blob,
        l1_data_gas_price: crate::v0_10_0::ResourcePrice {
            price_in_fri: Felt::from_hex("0x1").unwrap(),
            price_in_wei: Felt::from_hex("0x1").unwrap(),
        },
        l1_gas_price: crate::v0_10_0::ResourcePrice {
            price_in_fri: Felt::from_hex("0x10").unwrap(),
            price_in_wei: Felt::from_hex("0x10").unwrap(),
        },
        l2_gas_price: crate::v0_10_0::ResourcePrice {
            price_in_fri: Felt::from_hex("0x5").unwrap(),
            price_in_wei: Felt::from_hex("0x5").unwrap(),
        },
        block_number: 100,
        sequencer_address: Felt::from_hex("0x1234").unwrap(),
        starknet_version: "0.13.4".to_string(),
        timestamp: 1700000000,
    }
}

/// Helper function to create a test InvokeTxnV3 with proof_facts
fn create_test_invoke_v3_with_proof_facts(proof_facts: Option<Vec<Felt>>) -> InvokeTxnV3 {
    InvokeTxnV3 {
        inner: crate::v0_10_0::InvokeTxnV3 {
            sender_address: Felt::from_hex("0x1").unwrap(),
            calldata: vec![Felt::from_hex("0x2").unwrap()].into(),
            signature: vec![Felt::from_hex("0x3").unwrap()].into(),
            nonce: Felt::from_hex("0x4").unwrap(),
            resource_bounds: crate::v0_10_0::ResourceBoundsMapping {
                l1_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x10, max_price_per_unit: 0x1 },
                l2_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x20, max_price_per_unit: 0x2 },
                l1_data_gas: crate::v0_10_0::ResourceBounds { max_amount: 0x30, max_price_per_unit: 0x3 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: crate::v0_10_0::DaMode::L1,
            fee_data_availability_mode: crate::v0_10_0::DaMode::L1,
        },
        proof_facts,
    }
}

// Real Starknet token addresses for testing with realistic values
const PROOF_FACT_ETH: &str = "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
const PROOF_FACT_STRK: &str = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

#[test]
fn test_block_with_txs_and_proof_facts_serialization() {
    // Test serialization of BlockWithTxsAndProofFacts with proof_facts present
    let block = BlockWithTxsAndProofFacts {
        transactions: vec![TxnWithHashAndProofFacts {
            transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                create_test_invoke_v3_with_proof_facts(Some(vec![
                    Felt::from_hex(PROOF_FACT_ETH).unwrap(),
                    Felt::from_hex(PROOF_FACT_STRK).unwrap(),
                ])),
            )),
            transaction_hash: Felt::from_hex("0xabc").unwrap(),
        }],
        status: BlockStatus::AcceptedOnL2,
        block_header: create_test_block_header(),
    };

    let json = serde_json::to_string(&block).unwrap();

    // Verify proof_facts is included in serialization
    assert!(json.contains("proof_facts"));
    // Verify the actual proof_fact values are present (lowercase hex without leading zeros removed)
    assert!(json.contains("49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"));

    // Verify round-trip deserialization
    let deserialized: BlockWithTxsAndProofFacts = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.transactions.len(), 1);
    assert_eq!(deserialized.status, BlockStatus::AcceptedOnL2);
}

#[test]
fn test_block_with_txs_and_proof_facts_none_not_serialized() {
    // When proof_facts is None, it should NOT appear in serialized JSON
    let block = BlockWithTxsAndProofFacts {
        transactions: vec![TxnWithHashAndProofFacts {
            transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                create_test_invoke_v3_with_proof_facts(None), // No proof_facts
            )),
            transaction_hash: Felt::from_hex("0xabc").unwrap(),
        }],
        status: BlockStatus::AcceptedOnL2,
        block_header: create_test_block_header(),
    };

    let json = serde_json::to_string(&block).unwrap();

    // proof_facts should NOT be in the output when it's None
    assert!(!json.contains("proof_facts"));
}

#[test]
fn test_block_with_txs_and_proof_facts_mixed_transactions() {
    // Test block with mix of transactions: some with proof_facts, some without
    let block = BlockWithTxsAndProofFacts {
        transactions: vec![
            // INVOKE V3 with proof_facts
            TxnWithHashAndProofFacts {
                transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                    create_test_invoke_v3_with_proof_facts(Some(vec![Felt::from_hex("0x100").unwrap()])),
                )),
                transaction_hash: Felt::from_hex("0x1").unwrap(),
            },
            // INVOKE V3 without proof_facts
            TxnWithHashAndProofFacts {
                transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                    create_test_invoke_v3_with_proof_facts(None),
                )),
                transaction_hash: Felt::from_hex("0x2").unwrap(),
            },
            // INVOKE V1 (doesn't have proof_facts)
            TxnWithHashAndProofFacts {
                transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V1(crate::v0_10_0::InvokeTxnV1 {
                    sender_address: Felt::from_hex("0x1").unwrap(),
                    calldata: vec![].into(),
                    signature: vec![].into(),
                    nonce: Felt::from_hex("0x0").unwrap(),
                    max_fee: Felt::from_hex("0x1000").unwrap(),
                })),
                transaction_hash: Felt::from_hex("0x3").unwrap(),
            },
        ],
        status: BlockStatus::AcceptedOnL2,
        block_header: create_test_block_header(),
    };

    let json = serde_json::to_string(&block).unwrap();

    // Verify we have 3 transactions
    let deserialized: BlockWithTxsAndProofFacts = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.transactions.len(), 3);

    // First transaction should have proof_facts
    match &deserialized.transactions[0].transaction {
        TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(v3)) => {
            assert!(v3.proof_facts.is_some());
        }
        _ => panic!("Expected INVOKE V3"),
    }

    // Second transaction should NOT have proof_facts
    match &deserialized.transactions[1].transaction {
        TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(v3)) => {
            assert!(v3.proof_facts.is_none());
        }
        _ => panic!("Expected INVOKE V3"),
    }
}

#[test]
fn test_preconfirmed_block_with_txs_and_proof_facts() {
    // Test PreConfirmedBlockWithTxsAndProofFacts type
    let block = PreConfirmedBlockWithTxsAndProofFacts {
        transactions: vec![TxnWithHashAndProofFacts {
            transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                create_test_invoke_v3_with_proof_facts(Some(vec![Felt::from_hex("0x100").unwrap()])),
            )),
            transaction_hash: Felt::from_hex("0xabc").unwrap(),
        }],
        pre_confirmed_block_header: create_test_preconfirmed_block_header(),
    };

    let json = serde_json::to_string(&block).unwrap();
    assert!(json.contains("proof_facts"));

    // Verify round-trip
    let deserialized: PreConfirmedBlockWithTxsAndProofFacts = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.transactions.len(), 1);
}

#[test]
fn test_maybe_preconfirmed_block_with_txs_and_proof_facts_confirmed() {
    // Test MaybePreConfirmedBlockWithTxsAndProofFacts::Block variant
    let block = MaybePreConfirmedBlockWithTxsAndProofFacts::Block(BlockWithTxsAndProofFacts {
        transactions: vec![TxnWithHashAndProofFacts {
            transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                create_test_invoke_v3_with_proof_facts(Some(vec![Felt::from_hex("0x100").unwrap()])),
            )),
            transaction_hash: Felt::from_hex("0xabc").unwrap(),
        }],
        status: BlockStatus::AcceptedOnL1,
        block_header: create_test_block_header(),
    });

    let json = serde_json::to_string(&block).unwrap();

    // Should contain status (confirmed blocks have status)
    assert!(json.contains("AcceptedOnL1") || json.contains("ACCEPTED_ON_L1"));

    // Verify round-trip
    let deserialized: MaybePreConfirmedBlockWithTxsAndProofFacts = serde_json::from_str(&json).unwrap();
    match deserialized {
        MaybePreConfirmedBlockWithTxsAndProofFacts::Block(b) => {
            assert_eq!(b.transactions.len(), 1);
            assert_eq!(b.status, BlockStatus::AcceptedOnL1);
        }
        _ => panic!("Expected Block variant"),
    }
}

#[test]
fn test_maybe_preconfirmed_block_with_txs_and_proof_facts_preconfirmed() {
    // Test MaybePreConfirmedBlockWithTxsAndProofFacts::PreConfirmed variant
    let block = MaybePreConfirmedBlockWithTxsAndProofFacts::PreConfirmed(PreConfirmedBlockWithTxsAndProofFacts {
        transactions: vec![TxnWithHashAndProofFacts {
            transaction: TxnWithProofFacts::Invoke(InvokeTxnWithProofFacts::V3(
                create_test_invoke_v3_with_proof_facts(Some(vec![Felt::from_hex("0x200").unwrap()])),
            )),
            transaction_hash: Felt::from_hex("0xdef").unwrap(),
        }],
        pre_confirmed_block_header: create_test_preconfirmed_block_header(),
    });

    let json = serde_json::to_string(&block).unwrap();

    // PreConfirmed blocks should NOT have status field
    assert!(!json.contains("status"));

    // Verify round-trip
    let deserialized: MaybePreConfirmedBlockWithTxsAndProofFacts = serde_json::from_str(&json).unwrap();
    match deserialized {
        MaybePreConfirmedBlockWithTxsAndProofFacts::PreConfirmed(b) => {
            assert_eq!(b.transactions.len(), 1);
        }
        _ => panic!("Expected PreConfirmed variant"),
    }
}
