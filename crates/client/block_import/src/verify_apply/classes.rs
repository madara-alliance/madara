use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use dc_db::DeoxysBackend;
use dc_db::{bonsai_identifier, DeoxysStorageError};
use dp_state_update::DeclaredClassItem;
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};

// "CONTRACT_CLASS_LEAF_V0"
const CONTRACT_CLASS_HASH_VERSION: Felt = Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f4c4541465f5630");

pub fn class_trie_root(
    backend: &DeoxysBackend,
    declared_classes: &[DeclaredClassItem],
    block_number: u64,
) -> Result<Felt, DeoxysStorageError> {
    let mut class_trie = backend.class_trie();

    let updates: Vec<_> = declared_classes
        .into_par_iter()
        .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
            let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);
            (*class_hash, hash)
        })
        .collect();

    log::debug!("class_trie inserting");
    for (key, value) in updates {
        let bytes = key.to_bytes_be();
        let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        class_trie.insert(bonsai_identifier::CLASS, &bv, &value)?;
    }

    log::debug!("class_trie committing");
    class_trie.commit(BasicId::new(block_number))?;

    let root_hash = class_trie.root_hash(bonsai_identifier::CLASS)?;

    log::debug!("class_trie committed");

    Ok(root_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_class_hash_version() {
        assert_eq!(CONTRACT_CLASS_HASH_VERSION, Felt::from_bytes_be_slice(b"CONTRACT_CLASS_LEAF_V0"));
    }
}
