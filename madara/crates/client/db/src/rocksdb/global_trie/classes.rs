use crate::rocksdb::trie::WrappedBonsaiError;
use crate::{prelude::*, rocksdb::RocksDBStorage};
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use mp_state_update::{DeclaredClassItem, MigratedClassItem};
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

// "CONTRACT_CLASS_LEAF_V0"
const CONTRACT_CLASS_HASH_VERSION: Felt = Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f4c4541465f5630");

/// Computes the class trie leaf hash for a class.
/// The leaf hash is: Poseidon("CONTRACT_CLASS_LEAF_V0", compiled_class_hash)
fn compute_class_leaf_hash(compiled_class_hash: &Felt) -> Felt {
    Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash)
}

pub fn class_trie_root(
    backend: &RocksDBStorage,
    declared_classes: &[DeclaredClassItem],
    migrated_classes: &[MigratedClassItem],
    block_number: u64,
) -> Result<Felt> {
    let mut class_trie = backend.class_trie();

    // Process newly declared classes
    let declared_updates: Vec<_> = declared_classes
        .into_par_iter()
        .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
            let hash = compute_class_leaf_hash(compiled_class_hash);
            (*class_hash, hash)
        })
        .collect();

    // Process migrated classes (SNIP-34)
    // For migrated classes, the compiled_class_hash in MigratedClassItem is the new BLAKE hash
    let migrated_updates: Vec<_> = migrated_classes
        .into_par_iter()
        .map(|MigratedClassItem { class_hash, compiled_class_hash }| {
            let hash = compute_class_leaf_hash(compiled_class_hash);
            (*class_hash, hash)
        })
        .collect();

    tracing::trace!(
        "class_trie inserting {} declared classes, {} migrated classes",
        declared_updates.len(),
        migrated_updates.len()
    );

    for (key, value) in declared_updates.into_iter().chain(migrated_updates) {
        let bytes = key.to_bytes_be();
        let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        class_trie.insert(super::bonsai_identifier::CLASS, &bv, &value).map_err(WrappedBonsaiError)?;
    }

    tracing::trace!("class_trie committing");
    class_trie.commit(BasicId::new(block_number)).map_err(WrappedBonsaiError)?;

    let root_hash = class_trie.root_hash(super::bonsai_identifier::CLASS).map_err(WrappedBonsaiError)?;

    tracing::trace!("class_trie committed");

    Ok(root_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rocksdb::global_trie::tests::setup_test_backend, MadaraBackend};
    use rstest::*;
    use std::sync::Arc;

    #[test]
    fn test_contract_class_hash_version() {
        assert_eq!(CONTRACT_CLASS_HASH_VERSION, Felt::from_bytes_be_slice(b"CONTRACT_CLASS_LEAF_V0"));
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
        let result = class_trie_root(&backend.db, &declared_classes, &[], block_number).unwrap();

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
        let result = class_trie_root(&backend.db, &declared_classes, &migrated_classes, block_number).unwrap();

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
