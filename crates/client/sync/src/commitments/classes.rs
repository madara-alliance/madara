use blockifier::state::cached_state::CommitmentStateDiff;
use dc_db::storage_handler::DeoxysStorageError;
use dc_db::DeoxysBackend;
use dp_convert::ToFelt;
use rayon::prelude::*;
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
    csd: &CommitmentStateDiff,
    block_number: u64,
) -> Result<Felt, DeoxysStorageError> {
    let mut handler_class = backend.class_trie_mut();

    let updates = csd
        .class_hash_to_compiled_class_hash
        .iter()
        .par_bridge()
        .map(|(class_hash, compiled_class_hash)| {
            let compiled_class_hash = compiled_class_hash.to_felt();

            let hash = Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, &compiled_class_hash);

            (class_hash, hash)
        })
        .collect::<Vec<_>>();

    handler_class.init()?;
    handler_class.update(updates)?;
    handler_class.commit(block_number)?;
    log::debug!("committed class trie root");

    handler_class.root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_class_hash_version() {
        assert_eq!(CONTRACT_CLASS_HASH_VERSION, Felt::from_bytes_be_slice("CONTRACT_CLASS_LEAF_V0".as_bytes()));
    }
}
