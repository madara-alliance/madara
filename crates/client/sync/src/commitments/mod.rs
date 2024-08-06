mod classes;
mod contracts;
mod events;
mod receipts;
mod transactions;

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use classes::class_trie_root;
use contracts::contract_trie_root;
use dc_db::DeoxysBackend;
use dp_state_update::StateDiff;
pub use events::memory_event_commitment;
pub use receipts::memory_receipt_commitment;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};
pub use transactions::{calculate_transaction_hash, memory_transaction_commitment};

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt = Felt::from_hex_unchecked("0x535441524b4e45545f53544154455f5630");

/// Calculate state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `contracts_trie_root` - The root of the contracts trie.
/// * `classes_trie_root` - The root of the classes trie.
///
/// # Returns
///
/// The state commitment as a `Felt`.
pub fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt) -> Felt {
    if classes_trie_root == Felt::ZERO {
        contracts_trie_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
    }
}

/// Update the state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `CommitmentStateDiff` - The commitment state diff inducing unprocessed state changes.
/// * `BonsaiDb` - The database responsible for storing computing the state tries.
///
///
/// The updated state root as a `Felt`.
pub fn update_tries_and_compute_state_root(backend: &DeoxysBackend, state_diff: &StateDiff, block_number: u64) -> Felt {
    let StateDiff {
        storage_diffs,
        deprecated_declared_classes: _,
        declared_classes,
        deployed_contracts,
        replaced_classes,
        nonces,
    } = state_diff;

    // Update contract and its storage tries
    let (contract_trie_root, class_trie_root) = rayon::join(
        || {
            contract_trie_root(backend, deployed_contracts, replaced_classes, nonces, storage_diffs, block_number)
                .expect("Failed to compute contract root")
        },
        || class_trie_root(backend, declared_classes, block_number).expect("Failed to compute class root"),
    );

    calculate_state_root(contract_trie_root, class_trie_root)
}

/// Compute the root hash of a list of values.
// The `HashMapDb` can't fail, so we can safely unwrap the results.
pub fn compute_root<H>(values: &[Felt]) -> Felt
where
    H: StarkHash + Send + Sync,
{
    //TODO: replace the identifier by an empty slice when bonsai will support it
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    values.iter().enumerate().for_each(|(id, value)| {
        let key = BitVec::from_vec(id.to_be_bytes().to_vec());
        bonsai_storage.insert(IDENTIFIER, key.as_bitslice(), value).expect("Failed to insert into bonsai storage");
    });

    // Note that committing changes still has the greatest performance hit
    // as this is where the root hash is calculated. Due to the Merkle structure
    // of Bonsai Tries, this results in a trie size that grows very rapidly with
    // each new insertion. It seems that the only vector of optimization here
    // would be to optimize the tree traversal and hash computation.
    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    // run in a blocking-safe thread to avoid starving the thread pool
    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
    bonsai_storage.root_hash(IDENTIFIER).expect("Failed to get root hash")
}

pub fn compute_root_keyed<H>(entries: &[(Felt, Felt)]) -> Felt
where
    H: StarkHash + Send + Sync,
{
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config).expect("Failed to create bonsai storage");
    entries.iter().for_each(|(key, value)| {
        let bytes = key.to_bytes_be();
        let bv: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        bonsai_storage.insert(IDENTIFIER, &bv, value).expect("Failed to insert into bonsai storage");
    });

    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
    bonsai_storage.root_hash(IDENTIFIER).expect("Failed to get root hash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_root() {
        let values = vec![Felt::ONE, Felt::TWO, Felt::THREE];
        let root = compute_root::<Poseidon>(&values);

        assert_eq!(root, Felt::from_hex_unchecked("0x3b5cc7f1292eb3847c3f902d048a7e5dc7702d1c191ccd17c2d33f797e6fc32"));
    }
}
