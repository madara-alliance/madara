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
    let multiple = AddressFilter::Multiple(vec![
        Felt::from_hex("0x10").unwrap(),
        Felt::from_hex("0x20").unwrap(),
    ]);
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
// SimulateTransactionsResult Tests
// ============================================================================

#[test]
fn test_simulate_transactions_result_without_initial_reads() {
    // When initial_reads is None, it should not be serialized
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
    assert!(result.initial_reads.is_none());
}

#[test]
fn test_simulate_transactions_result_with_initial_reads() {
    // When RETURN_INITIAL_READS flag is set, initial_reads should be present
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
        },
        "initial_reads": {
            "storage": [{"contract_address": "0x1", "key": "0x2", "value": "0x3"}],
            "nonces": [],
            "class_hashes": [],
            "declared_contracts": []
        }
    }"#;

    let result: SimulateTransactionsResult = serde_json::from_str(result_json).unwrap();
    assert!(result.initial_reads.is_some());
    let reads = result.initial_reads.unwrap();
    assert_eq!(reads.storage.len(), 1);
}

// ============================================================================
// TraceBlockTransactionsResult Tests
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
    assert!(result.initial_reads.is_none());
    assert_eq!(result.transaction_hash, Felt::from_hex("0x123").unwrap());
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
        address: Some(AddressFilter::Multiple(vec![
            Felt::from_hex("0x10").unwrap(),
            Felt::from_hex("0x20").unwrap(),
        ])),
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
