use std::sync::Arc;

use bonsai_trie::id::BasicIdBuilder;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mc_db::bonsai_db::BonsaiDb;
use mp_felt::Felt252Wrapper;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use sp_runtime::traits::Block as BlockT;
use starknet_types_core::hash::Poseidon;

use super::lib::ClassCommitmentLeafHash;

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
pub fn calculate_class_commitment_leaf_hash<H: HasherT>(
    compiled_class_hash: Felt252Wrapper,
) -> ClassCommitmentLeafHash {
    let contract_class_hash_version = Felt252Wrapper::try_from("CONTRACT_CLASS_LEAF_V0".as_bytes()).unwrap(); // Unwrap safu

    let hash = H::compute_hash_on_elements(&[contract_class_hash_version.0, compiled_class_hash.0]);

    hash.into()
}

/// Calculate class commitment trie root hash value.
///
/// The classes trie encodes the information about the existing classes in the state of Starknet.
/// It maps (Cairo 1.0) class hashes to their compiled class hashes
///
/// # Arguments
///
/// * `classes` - The classes to get the root from.
///
/// # Returns
///
/// The merkle root of the merkle trie built from the classes.
pub fn class_trie_root<B, H>(
    class_hash: Felt252Wrapper,
    compiled_class_hash: Felt252Wrapper,
    backend: &Arc<BonsaiDb<B>>,
) -> Felt252Wrapper
where
    B: BlockT,
    H: HasherT,
{
    // let config = BonsaiStorageConfig::default();
    // let bonsai_db = backend.as_ref();
    // let mut bonsai_storage =
    //     BonsaiStorage::<_, _, Poseidon>::new(bonsai_db, config).expect("Failed to create bonsai
    // storage");

    // let class_commitment_leaf_hash = calculate_class_commitment_leaf_hash(compiled_class_hash);

    // bonsai_storage
    //     .insert(class_hash.as_bitslice(), &class_commitment_leaf_hash.into())
    //     .expect("Failed to insert into bonsai storage");

    // let id_builder = BasicIdBuilder::new();
    // let id = id_builder.new_id();
    // bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    // let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    // Ok(Felt252Wrapper::from(root_hash))
    Felt252Wrapper::default()
}
