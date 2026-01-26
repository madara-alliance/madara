//! Verification logic to compare Rust execution results with Blockifier results.

use starknet_types_core::felt::Felt;

use crate::types::{Event, ExecutionResult, StateDiff};

/// Result of comparing Rust and Blockifier execution.
#[derive(Debug)]
pub struct VerificationResult {
    /// Whether all checks passed
    pub passed: bool,
    /// List of mismatches found
    pub errors: Vec<VerificationError>,
}

impl VerificationResult {
    /// Create a passing result with no errors.
    pub fn pass() -> Self {
        Self { passed: true, errors: Vec::new() }
    }

    /// Create a failing result with errors.
    pub fn fail(errors: Vec<VerificationError>) -> Self {
        Self { passed: false, errors }
    }
}

/// Types of verification errors.
#[derive(Debug)]
pub enum VerificationError {
    /// Storage value mismatch
    StorageMismatch { contract: Felt, key: Felt, rust_value: Felt, blockifier_value: Felt },

    /// Rust has extra storage write not in Blockifier
    ExtraStorageInRust { contract: Felt, key: Felt },

    /// Blockifier has storage write not in Rust
    MissingStorageInRust { contract: Felt, key: Felt, blockifier_value: Felt },

    /// Return data mismatch
    RetdataMismatch { rust: Vec<Felt>, blockifier: Vec<Felt> },

    /// Event count mismatch
    EventCountMismatch { rust_count: usize, blockifier_count: usize },

    /// Event content mismatch
    EventMismatch { index: usize, rust: Event, blockifier_keys: Vec<Felt>, blockifier_data: Vec<Felt> },

    /// Success/failure status mismatch
    StatusMismatch { rust_failed: bool, blockifier_failed: bool },

    /// Nonce mismatch
    NonceMismatch { contract: Felt, rust: Felt, blockifier: Felt },

    /// Rust has extra nonce update not in Blockifier
    ExtraNonceInRust { contract: Felt, rust_nonce: Felt },

    /// Blockifier has nonce update not in Rust
    MissingNonceInRust { contract: Felt, blockifier_nonce: Felt },

    /// L2-to-L1 message count mismatch
    MessageCountMismatch { rust_count: usize, blockifier_count: usize },

    /// L2-to-L1 message content mismatch
    MessageMismatch { index: usize, rust_to: Felt, rust_payload: Vec<Felt>, blockifier_to: Felt, blockifier_payload: Vec<Felt> },

    /// Class hash deployment/replacement mismatch
    ClassHashMismatch { contract: Felt, rust: Option<Felt>, blockifier: Option<Felt> },

    /// Compiled class hash mismatch
    CompiledClassHashMismatch { class_hash: Felt, rust: Option<Felt>, blockifier: Option<Felt> },
}

/// Compare state diffs from Rust and Blockifier.
pub fn compare_state_diffs(
    rust: &StateDiff,
    blockifier_storage: &[(Felt, Vec<(Felt, Felt)>)],
) -> Vec<VerificationError> {
    let mut errors = Vec::new();

    // Convert blockifier storage to a map for easier lookup
    let mut blockifier_map: std::collections::HashMap<(Felt, Felt), Felt> = std::collections::HashMap::new();
    for (contract, entries) in blockifier_storage {
        for (key, value) in entries {
            blockifier_map.insert((*contract, *key), *value);
        }
    }

    // Check all Rust writes exist in Blockifier with same value
    for (contract, entries) in &rust.storage_updates {
        for (key, rust_value) in entries {
            match blockifier_map.get(&(contract.0, key.0)) {
                Some(blockifier_value) if blockifier_value == rust_value => {
                    // Match - good
                }
                Some(blockifier_value) => {
                    errors.push(VerificationError::StorageMismatch {
                        contract: contract.0,
                        key: key.0,
                        rust_value: *rust_value,
                        blockifier_value: *blockifier_value,
                    });
                }
                None => {
                    errors.push(VerificationError::ExtraStorageInRust { contract: contract.0, key: key.0 });
                }
            }
        }
    }

    // Check for Blockifier writes missing in Rust
    for ((contract, key), blockifier_value) in &blockifier_map {
        let in_rust = rust
            .storage_updates
            .iter()
            .any(|(c, entries)| c.0 == *contract && entries.iter().any(|(k, _)| k.0 == *key));

        if !in_rust {
            errors.push(VerificationError::MissingStorageInRust {
                contract: *contract,
                key: *key,
                blockifier_value: *blockifier_value,
            });
        }
    }

    errors
}

/// Compare events from Rust and Blockifier.
pub fn compare_events(
    rust_events: &[Event],
    blockifier_events: &[(Vec<Felt>, Vec<Felt>)], // (keys, data) pairs
) -> Vec<VerificationError> {
    let mut errors = Vec::new();

    if rust_events.len() != blockifier_events.len() {
        errors.push(VerificationError::EventCountMismatch {
            rust_count: rust_events.len(),
            blockifier_count: blockifier_events.len(),
        });
        return errors;
    }

    for (i, (rust_event, (blockifier_keys, blockifier_data))) in
        rust_events.iter().zip(blockifier_events.iter()).enumerate()
    {
        if rust_event.keys != *blockifier_keys || rust_event.data != *blockifier_data {
            errors.push(VerificationError::EventMismatch {
                index: i,
                rust: rust_event.clone(),
                blockifier_keys: blockifier_keys.clone(),
                blockifier_data: blockifier_data.clone(),
            });
        }
    }

    errors
}

/// Compare nonces from Rust and Blockifier.
pub fn compare_nonces(
    rust: &StateDiff,
    blockifier_nonces: &[(Felt, Felt)], // (contract, nonce) pairs
) -> Vec<VerificationError> {
    let mut errors = Vec::new();

    // Convert blockifier nonces to a map for easier lookup
    let blockifier_map: std::collections::HashMap<Felt, Felt> = blockifier_nonces.iter().copied().collect();

    // Check all Rust nonce updates exist in Blockifier with same value
    for (contract, rust_nonce) in &rust.address_to_nonce {
        match blockifier_map.get(&contract.0) {
            Some(blockifier_nonce) if blockifier_nonce == &rust_nonce.0 => {
                // Match - good
            }
            Some(blockifier_nonce) => {
                errors.push(VerificationError::NonceMismatch {
                    contract: contract.0,
                    rust: rust_nonce.0,
                    blockifier: *blockifier_nonce,
                });
            }
            None => {
                errors.push(VerificationError::ExtraNonceInRust { contract: contract.0, rust_nonce: rust_nonce.0 });
            }
        }
    }

    // Check for Blockifier nonce updates missing in Rust
    for (contract, blockifier_nonce) in &blockifier_map {
        let in_rust = rust.address_to_nonce.iter().any(|(c, _)| c.0 == *contract);

        if !in_rust {
            errors.push(VerificationError::MissingNonceInRust {
                contract: *contract,
                blockifier_nonce: *blockifier_nonce,
            });
        }
    }

    errors
}

/// Compare L2-to-L1 messages from Rust and Blockifier.
pub fn compare_l2_to_l1_messages(
    rust_messages: &[crate::types::L2ToL1Message],
    blockifier_messages: &[(Felt, Vec<Felt>)], // (to_address, payload) pairs
) -> Vec<VerificationError> {
    let mut errors = Vec::new();

    if rust_messages.len() != blockifier_messages.len() {
        errors.push(VerificationError::MessageCountMismatch {
            rust_count: rust_messages.len(),
            blockifier_count: blockifier_messages.len(),
        });
        return errors;
    }

    for (i, (rust_msg, (blockifier_to, blockifier_payload))) in
        rust_messages.iter().zip(blockifier_messages.iter()).enumerate()
    {
        if rust_msg.to_address != *blockifier_to || rust_msg.payload != *blockifier_payload {
            errors.push(VerificationError::MessageMismatch {
                index: i,
                rust_to: rust_msg.to_address,
                rust_payload: rust_msg.payload.clone(),
                blockifier_to: *blockifier_to,
                blockifier_payload: blockifier_payload.clone(),
            });
        }
    }

    errors
}

/// Compare class hash deployments/replacements from Rust and Blockifier.
pub fn compare_class_hashes(
    rust: &StateDiff,
    blockifier_class_hashes: &[(Felt, Felt)], // (contract, class_hash) pairs
) -> Vec<VerificationError> {
    let mut errors = Vec::new();

    // Convert to maps for easier comparison
    let rust_map: std::collections::HashMap<Felt, Felt> =
        rust.address_to_class_hash.iter().map(|(addr, hash)| (addr.0, *hash)).collect();
    let blockifier_map: std::collections::HashMap<Felt, Felt> = blockifier_class_hashes.iter().copied().collect();

    // Check all contracts
    let all_contracts: std::collections::HashSet<Felt> =
        rust_map.keys().chain(blockifier_map.keys()).copied().collect();

    for contract in all_contracts {
        let rust_class = rust_map.get(&contract).copied();
        let blockifier_class = blockifier_map.get(&contract).copied();

        if rust_class != blockifier_class {
            errors.push(VerificationError::ClassHashMismatch { contract, rust: rust_class, blockifier: blockifier_class });
        }
    }

    errors
}

/// Compare compiled class hashes from Rust and Blockifier.
pub fn compare_compiled_class_hashes(
    rust: &StateDiff,
    blockifier_compiled: &[(Felt, Felt)], // (class_hash, compiled_class_hash) pairs
) -> Vec<VerificationError> {
    let mut errors = Vec::new();

    // Convert to maps for easier comparison
    let rust_map: std::collections::HashMap<Felt, Felt> =
        rust.class_hash_to_compiled_class_hash.iter().map(|(hash, compiled)| (*hash, *compiled)).collect();
    let blockifier_map: std::collections::HashMap<Felt, Felt> = blockifier_compiled.iter().copied().collect();

    // Check all class hashes
    let all_classes: std::collections::HashSet<Felt> =
        rust_map.keys().chain(blockifier_map.keys()).copied().collect();

    for class_hash in all_classes {
        let rust_compiled = rust_map.get(&class_hash).copied();
        let blockifier_compiled = blockifier_map.get(&class_hash).copied();

        if rust_compiled != blockifier_compiled {
            errors.push(VerificationError::CompiledClassHashMismatch {
                class_hash,
                rust: rust_compiled,
                blockifier: blockifier_compiled,
            });
        }
    }

    errors
}

/// Full comparison of Rust execution result with Blockifier output.
///
/// This is the main entry point for verification.
pub fn verify_execution(
    rust_result: &ExecutionResult,
    blockifier_storage: &[(Felt, Vec<(Felt, Felt)>)],
    blockifier_retdata: &[Felt],
    blockifier_events: &[(Vec<Felt>, Vec<Felt>)],
    blockifier_failed: bool,
) -> VerificationResult {
    verify_execution_comprehensive(
        rust_result,
        blockifier_storage,
        blockifier_retdata,
        blockifier_events,
        blockifier_failed,
        &[], // nonces
        &[], // l2_to_l1_messages
        &[], // class_hashes
        &[], // compiled_class_hashes
    )
}

/// Comprehensive verification including all state changes.
///
/// This is the enhanced entry point that checks everything.
pub fn verify_execution_comprehensive(
    rust_result: &ExecutionResult,
    blockifier_storage: &[(Felt, Vec<(Felt, Felt)>)],
    blockifier_retdata: &[Felt],
    blockifier_events: &[(Vec<Felt>, Vec<Felt>)],
    blockifier_failed: bool,
    blockifier_nonces: &[(Felt, Felt)],
    blockifier_messages: &[(Felt, Vec<Felt>)],
    blockifier_class_hashes: &[(Felt, Felt)],
    blockifier_compiled_class_hashes: &[(Felt, Felt)],
) -> VerificationResult {
    let mut all_errors = Vec::new();

    // 1. Compare storage updates
    let storage_errors = compare_state_diffs(&rust_result.state_diff, blockifier_storage);
    all_errors.extend(storage_errors);

    // 2. Compare return data
    if rust_result.call_result.retdata != blockifier_retdata {
        all_errors.push(VerificationError::RetdataMismatch {
            rust: rust_result.call_result.retdata.clone(),
            blockifier: blockifier_retdata.to_vec(),
        });
    }

    // 3. Compare events
    let event_errors = compare_events(&rust_result.call_result.events, blockifier_events);
    all_errors.extend(event_errors);

    // 4. Compare success/failure status
    if rust_result.call_result.failed != blockifier_failed {
        all_errors
            .push(VerificationError::StatusMismatch { rust_failed: rust_result.call_result.failed, blockifier_failed });
    }

    // 5. Compare nonces
    if !blockifier_nonces.is_empty() {
        let nonce_errors = compare_nonces(&rust_result.state_diff, blockifier_nonces);
        all_errors.extend(nonce_errors);
    }

    // 6. Compare L2-to-L1 messages
    if !blockifier_messages.is_empty() {
        let message_errors = compare_l2_to_l1_messages(&rust_result.call_result.l2_to_l1_messages, blockifier_messages);
        all_errors.extend(message_errors);
    }

    // 7. Compare class hash deployments/replacements
    if !blockifier_class_hashes.is_empty() {
        let class_hash_errors = compare_class_hashes(&rust_result.state_diff, blockifier_class_hashes);
        all_errors.extend(class_hash_errors);
    }

    // 8. Compare compiled class hashes
    if !blockifier_compiled_class_hashes.is_empty() {
        let compiled_errors = compare_compiled_class_hashes(&rust_result.state_diff, blockifier_compiled_class_hashes);
        all_errors.extend(compiled_errors);
    }

    if all_errors.is_empty() {
        VerificationResult::pass()
    } else {
        VerificationResult::fail(all_errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContractAddress, StorageKey};

    #[test]
    fn test_matching_state_diffs() {
        let mut rust_diff = StateDiff::default();
        rust_diff
            .storage_updates
            .entry(ContractAddress(Felt::from(1u64)))
            .or_default()
            .insert(StorageKey(Felt::from(100u64)), Felt::from(42u64));

        let blockifier_storage = vec![(Felt::from(1u64), vec![(Felt::from(100u64), Felt::from(42u64))])];

        let errors = compare_state_diffs(&rust_diff, &blockifier_storage);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_mismatched_state_diffs() {
        let mut rust_diff = StateDiff::default();
        rust_diff
            .storage_updates
            .entry(ContractAddress(Felt::from(1u64)))
            .or_default()
            .insert(StorageKey(Felt::from(100u64)), Felt::from(42u64));

        let blockifier_storage = vec![(
            Felt::from(1u64),
            vec![(Felt::from(100u64), Felt::from(99u64))], // Different value
        )];

        let errors = compare_state_diffs(&rust_diff, &blockifier_storage);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], VerificationError::StorageMismatch { .. }));
    }
}
