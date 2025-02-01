use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use mc_db::MadaraBackend;
use mc_db::{bonsai_identifier, MadaraStorageError};
use mp_state_update::DeclaredClassItem;
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

// "CONTRACT_CLASS_LEAF_V0"
const CONTRACT_CLASS_HASH_VERSION: Felt = Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f4c4541465f5630");

pub fn class_trie_root(
    backend: &MadaraBackend,
    declared_classes: &[DeclaredClassItem],
    block_number: u64,
) -> Result<Felt, MadaraStorageError> {
    let mut class_trie = backend.class_trie();

    let updates: Vec<_> = declared_classes
        .into_par_iter()
        .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
            let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);
            (*class_hash, hash)
        })
        .collect();

    tracing::trace!("class_trie inserting");
    for (key, value) in updates {
        let bytes = key.to_bytes_be();
        let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        class_trie.insert(bonsai_identifier::CLASS, &bv, &value)?;
    }

    tracing::trace!("class_trie committing");
    class_trie.commit(BasicId::new(block_number))?;

    let root_hash = class_trie.root_hash(bonsai_identifier::CLASS)?;

    tracing::trace!("class_trie committed");

    Ok(root_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_apply::verify_apply_tests::setup_test_backend;
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
        let result = class_trie_root(&backend, &declared_classes, block_number).unwrap();

        // Assert that the resulting root hash matches the expected value
        assert_eq!(
            result,
            Felt::from_hex_unchecked("0x9e521cb5e73189fe985db9dfd50b1dcdefc95ca4e1ebf23b0a4408a81bb610")
        );
    }
}
