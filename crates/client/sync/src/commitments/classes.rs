use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use dc_db::storage_handler::{DeoxysStorageError, StorageType};
use dc_db::DeoxysBackend;
use dp_convert::ToFelt;
use indexmap::IndexMap;
use rayon::prelude::*;
use starknet_api::core::{ClassHash, CompiledClassHash};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

// "CONTRACT_CLASS_LEAF_V0"
const CONTRACT_CLASS_HASH_VERSION: Felt =
    Felt::from_raw([115292049744600508, 18444375821049509847, 12057587991035439952, 9331882290187415277]);

/// Calculates the class trie root
///
/// # Arguments
///
/// * `csd`          - Commitment state diff for the current block.
/// * `bonsai_class` - Bonsai db used to store class hashes.
/// * `block_number` - The current block number.
///
/// # Returns
///
/// The class root.
pub fn class_trie_root(
    backend: &DeoxysBackend,
    class_hash_to_compiled_class_hash: IndexMap<ClassHash, CompiledClassHash>,
    block_number: u64,
) -> Result<Felt, DeoxysStorageError> {
    let mut class_trie = backend.class_trie();

    let updates: Vec<_> = class_hash_to_compiled_class_hash
        .into_par_iter()
        .map(|(class_hash, compiled_class_hash)| {
            let compiled_class_hash = compiled_class_hash.to_felt();

            let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, &compiled_class_hash);

            (class_hash, hash)
        })
        .collect();

    for (key, value) in updates {
        class_trie
            .insert(&[], &key.0.bytes().as_bits()[5..], &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?
    }

    class_trie
        .commit(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;

    log::debug!("committed class trie root");

    let root_hash =
        class_trie.root_hash(&[]).map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;

    Ok(root_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_class_hash_version() {
        assert_eq!(CONTRACT_CLASS_HASH_VERSION, Felt::from_bytes_be_slice("CONTRACT_CLASS_LEAF_V0".as_bytes()));
    }
}
