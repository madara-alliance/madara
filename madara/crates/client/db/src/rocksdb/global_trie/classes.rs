use super::shared;
use super::ClassTrieTimings;
use crate::metrics::metrics;
use crate::{prelude::*, rocksdb::RocksDBStorage};
use mp_state_update::{DeclaredClassItem, MigratedClassItem};
use starknet_types_core::felt::Felt;

pub fn class_trie_root(
    backend: &RocksDBStorage,
    declared_classes: &[DeclaredClassItem],
    migrated_classes: &[MigratedClassItem],
    block_number: u64,
) -> Result<(Felt, ClassTrieTimings)> {
    let mut class_trie = backend.class_trie();
    let class_updates = shared::collect_class_updates(declared_classes, migrated_classes);

    tracing::trace!(
        "class_trie inserting {} declared classes, {} migrated classes",
        declared_classes.len(),
        migrated_classes.len()
    );

    tracing::trace!("class_trie committing");
    let (root_hash, timings) = shared::class_trie_root_from_updates(&mut class_trie, class_updates, block_number)?;
    let class_commit_secs = timings.trie_commit.as_secs_f64();
    metrics().class_trie_commit_duration.record(class_commit_secs, &[]);
    metrics().class_trie_commit_last.record(class_commit_secs, &[]);

    tracing::trace!("class_trie committed");

    Ok((root_hash, timings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rocksdb::global_trie::tests::setup_test_backend, MadaraBackend};
    use rstest::*;
    use std::sync::Arc;

    #[test]
    fn test_contract_class_hash_version() {
        assert_eq!(shared::CONTRACT_CLASS_HASH_VERSION, Felt::from_bytes_be_slice(b"CONTRACT_CLASS_LEAF_V0"));
    }

    #[rstest]
    fn test_class_trie_root(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;
        // Create sample DeclaredClassItems with predefined class and compiled class hashes
        let declared_classes = vec![
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked(
                    "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                ),
                compiled_class_hash: Felt::from_hex_unchecked(
                    "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
                ),
            },
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked(
                    "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                ),
                compiled_class_hash: Felt::from_hex_unchecked(
                    "0x1234567890abcdeffedcba09876543211234567890abcdeffedcba0987654321",
                ),
            },
        ];

        // Set the block number for the test
        let block_number = 1;

        // Call the class_trie_root function with the test data
        let (result, _timings) = class_trie_root(&backend.db, &declared_classes, &[], block_number).unwrap();

        // Assert that the resulting root hash matches the expected value
        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x9e521cb5e73189fe985db9dfd50b1dcdefc95ca4e1ebf23b0a4408a81bb610")
        );
    }

    /// Test that migrated_compiled_classes are correctly incorporated into the class trie.
    /// This validates SNIP-34 migration handling where existing classes get new BLAKE hashes.
    #[rstest]
    fn test_class_trie_root_with_migrated_classes(setup_test_backend: Arc<MadaraBackend>) {
        let backend = setup_test_backend;

        // Create a newly declared class
        let declared_classes = vec![DeclaredClassItem {
            class_hash: Felt::from_hex_unchecked("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"),
            compiled_class_hash: Felt::from_hex_unchecked(
                "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
            ),
        }];

        // Create a migrated class (SNIP-34)
        let migrated_classes = vec![MigratedClassItem {
            class_hash: Felt::from_hex_unchecked("0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"),
            compiled_class_hash: Felt::from_hex_unchecked(
                "0x1234567890abcdeffedcba09876543211234567890abcdeffedcba0987654321",
            ),
        }];

        let block_number = 1;

        // Call the class_trie_root function with both declared and migrated classes
        let (result, _timings) =
            class_trie_root(&backend.db, &declared_classes, &migrated_classes, block_number).unwrap();

        // The result should incorporate both declared and migrated classes.
        // The expected hash is computed by inserting both class leaf hashes into the trie.
        // Since this test uses the same class_hash/compiled_class_hash pairs as the original test
        // (just split between declared and migrated), the root hash should be the same.
        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x9e521cb5e73189fe985db9dfd50b1dcdefc95ca4e1ebf23b0a4408a81bb610")
        );
    }
}
