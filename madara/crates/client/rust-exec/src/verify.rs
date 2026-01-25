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
    let mut all_errors = Vec::new();

    // 1. Compare state diff
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
