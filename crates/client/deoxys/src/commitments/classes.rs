use std::sync::Arc;

use bitvec::vec::BitVec;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbError;
use mp_felt::Felt252Wrapper;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use sp_runtime::traits::Block as BlockT;
use starknet_types_core::hash::Poseidon;

/// Calculate class commitment trie leaf hash value.
///
/// See: <https://docs.starknet.io/documentation/architecture_and_concepts/State/starknet-state/#classes_trie>
///
/// # Arguments
///
/// * `compiled_class_hash` - The hash of the compiled class.
///
/// # Returns
///
/// The hash of the class commitment trie leaf.
pub fn calculate_class_commitment_leaf_hash<H: HasherT>(compiled_class_hash: Felt252Wrapper) -> Felt252Wrapper {
    let contract_class_hash_version = Felt252Wrapper::try_from("CONTRACT_CLASS_LEAF_V0".as_bytes()).unwrap(); // Unwrap safu

    let hash = H::compute_hash_on_elements(&[contract_class_hash_version.0, compiled_class_hash.0]);

    hash.into()
}

/// Update class trie root hash value with the new class definition.
///
/// The classes trie encodes the information about the existing classes in the state of Starknet.
/// It maps (Cairo 1.0) class hashes to their compiled class hashes
///
/// # Arguments
///
/// * `class_hash` - The hash of the class.
/// * `compiled_class_hash` - The hash of the compiled class.
/// * `bonsai_db` - The bonsai database responsible to compute the tries
///
/// # Returns
///
/// The class trie root hash represented as a `Felt252Wrapper` or a `BonsaiDbError`.
pub fn update_class_trie<B: BlockT>(
    class_hash: Felt252Wrapper,
    compiled_class_hash: Felt252Wrapper,
    backend: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = backend.as_ref();

    let mut bonsai_storage =
        BonsaiStorage::<_, _, Poseidon>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    // println!("Current column is: {:?}", bonsai_db.current_column);
    let class_commitment_leaf_hash = calculate_class_commitment_leaf_hash::<PoseidonHasher>(compiled_class_hash);
    let key = BitVec::from_vec(class_hash.0.to_bytes_be()[..31].to_vec());
    bonsai_storage
        .insert(key.as_bitslice(), &class_commitment_leaf_hash.into())
        .expect("Failed to insert into bonsai storage");

    let mut id_builder = BasicIdBuilder::new();
    let id = id_builder.new_id();
    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}

/// Get the actual class trie root hash.
///
/// # Arguments
///
/// * `bonsai_db` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The class trie root hash as a `Felt252Wrapper` or a `BonsaiDbError`.
pub fn get_class_trie_root<B: BlockT>(backend: &Arc<BonsaiDb<B>>) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = backend.as_ref();
    let bonsai_storage: BonsaiStorage<BasicId, &BonsaiDb<B>, Poseidon> =
        BonsaiStorage::<_, _, Poseidon>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    // println!("Current column is: {:?}", bonsai_db.current_column);

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}
