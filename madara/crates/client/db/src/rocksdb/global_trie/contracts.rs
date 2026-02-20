use super::shared;
use super::ContractTrieTimings;
use crate::metrics::metrics;
use crate::{prelude::*, rocksdb::RocksDBStorage};
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem};
use starknet_types_core::felt::Felt;

/// Calculates the contract trie root
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `block_number`    - The current block number.
///
/// # Returns
///
/// The contract root and timing information.
pub fn contract_trie_root(
    backend: &RocksDBStorage,
    deployed_contracts: &[DeployedContractItem],
    replaced_classes: &[ReplacedClassItem],
    nonces: &[NonceUpdate],
    storage_diffs: &[ContractStorageDiffItem],
    block_number: u64,
) -> Result<(Felt, ContractTrieTimings)> {
    let mut contract_storage_trie = backend.contract_storage_trie();
    let mut contract_trie = backend.contract_trie();

    tracing::trace!("contract_storage_trie inserting");
    tracing::trace!("contract_storage_trie commit");
    tracing::trace!("contract_trie committing");
    let (root_hash, timings) = shared::contract_trie_root_from_parts(
        backend,
        &mut contract_storage_trie,
        &mut contract_trie,
        shared::ContractTrieInputRefs {
            deployed_contracts,
            replaced_classes,
            nonces,
            storage_diffs,
            block_n: block_number,
        },
    )?;

    let storage_commit_secs = timings.storage_commit.as_secs_f64();
    metrics().contract_storage_trie_commit_duration.record(storage_commit_secs, &[]);
    metrics().contract_storage_trie_commit_last.record(storage_commit_secs, &[]);
    let contract_commit_secs = timings.trie_commit.as_secs_f64();
    metrics().contract_trie_commit_duration.record(contract_commit_secs, &[]);
    metrics().contract_trie_commit_last.record(contract_commit_secs, &[]);

    tracing::trace!("contract_trie committed");

    Ok((root_hash, timings))
}

#[cfg(test)]
mod contract_trie_root_tests {
    use super::*;
    use crate::{rocksdb::global_trie::tests::setup_test_backend, MadaraBackend};
    use mp_chain_config::ChainConfig;
    use mp_state_update::StorageEntry;
    use rstest::*;
    use std::sync::Arc;

    #[rstest]
    fn test_contract_trie_root_success(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        // Create dummy data
        let deployed_contracts = vec![DeployedContractItem {
            address: Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
            class_hash: Felt::from_hex_unchecked("0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321"),
        }];

        let replaced_classes = vec![ReplacedClassItem {
            contract_address: Felt::from_hex_unchecked(
                "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            ),
            class_hash: Felt::from_hex_unchecked("0x1234567890abcdeffedcba09876543211234567890abcdeffedcba0987654321"),
        }];

        let nonces = vec![NonceUpdate {
            contract_address: Felt::from_hex_unchecked(
                "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            ),
            nonce: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"),
        }];

        let storage_diffs = vec![ContractStorageDiffItem {
            address: Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
            storage_entries: vec![StorageEntry {
                key: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001"),
                value: Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000002"),
            }],
        }];

        let block_number = 1;

        // Call the function and print the result
        let (result, _timings) = contract_trie_root(
            &backend.db,
            &deployed_contracts,
            &replaced_classes,
            &nonces,
            &storage_diffs,
            block_number,
        )
        .unwrap();

        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x59b89ceac43986727fb4a57bd9f74690b5b3b0e976e7af0b10213c3d4392ef2")
        );
    }

    #[test]
    fn test_contract_state_leaf_hash_success() {
        let chain_config = Arc::new(ChainConfig::madara_test());
        let backend = MadaraBackend::open_for_testing(chain_config.clone());

        // Create dummy data
        let contract_address =
            Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");
        let contract_leaf = shared::ContractLeafState {
            class_hash: Some(Felt::from_hex_unchecked(
                "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
            )),
            storage_root: Some(Felt::from_hex_unchecked(
                "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            )),
            nonce: Some(Felt::from_hex_unchecked("0x0000000000000000000000000000000000000000000000000000000000000001")),
        };

        // Call the function and print the result
        let result =
            shared::contract_state_leaf_hash(&backend.db, &contract_address, &contract_leaf, /* block_number */ 0)
                .unwrap();
        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x6bbd8d4b5692148f83c38e19091f64381b5239e2a73f53b59be3ec3efb41143")
        );
    }
}
