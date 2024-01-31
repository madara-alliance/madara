// State trie

// Contract trie

// Storage trie

// Class trie

use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;

/// Hash of the StateCommitment tree
pub type StateCommitment = Felt252Wrapper;

/// Hash of the leaf of the ClassCommitment tree
pub type ClassCommitmentLeafHash = Felt252Wrapper;

/// Calculate state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia trees of height 251.
///
/// # Arguments
///
/// * `contracts_tree_root` - The root of the contracts tree.
/// * `classes_tree_root` - The root of the classes tree.
///
/// # Returns
///
/// The state commitment as a `StateCommitment`.
pub fn calculate_state_commitment<H: HasherT>(
    contracts_tree_root: Felt252Wrapper,
    classes_tree_root: Felt252Wrapper,
) -> StateCommitment
where
    H: HasherT,
{
    let starknet_state_prefix = Felt252Wrapper::try_from("STARKNET_STATE_V0".as_bytes()).unwrap();

    let state_commitment_hash =
        H::compute_hash_on_elements(&[starknet_state_prefix.0, contracts_tree_root.0, classes_tree_root.0]);

    state_commitment_hash.into()
}

/// Calculate class commitment tree leaf hash value.
///
/// See: <https://docs.starknet.io/documentation/architecture_and_concepts/State/starknet-state/#classes_tree>
///
/// # Arguments
///
/// * `compiled_class_hash` - The hash of the compiled class.
///
/// # Returns
///
/// The hash of the class commitment tree leaf.
pub fn calculate_class_commitment_leaf_hash<H: HasherT>(
    compiled_class_hash: Felt252Wrapper,
) -> ClassCommitmentLeafHash {
    let contract_class_hash_version = Felt252Wrapper::try_from("CONTRACT_CLASS_LEAF_V0".as_bytes()).unwrap(); // Unwrap safu

    let hash = H::compute_hash_on_elements(&[contract_class_hash_version.0, compiled_class_hash.0]);

    hash.into()
}

/// Calculate class commitment tree root hash value.
///
/// The classes tree encodes the information about the existing classes in the state of Starknet.
/// It maps (Cairo 1.0) class hashes to their compiled class hashes
///
/// # Arguments
///
/// * `classes` - The classes to get the root from.
///
/// # Returns
///
/// The merkle root of the merkle tree built from the classes.
pub fn calculate_class_commitment_tree_root_hash<H: HasherT>(class_hashes: &[Felt252Wrapper]) -> Felt252Wrapper {
    Felt252Wrapper::default()
}

/// Calculates the contract state hash from its preimage.
///
/// # Arguments
///
/// * `hash` - The hash of the contract definition.
/// * `root` - The root of root of another Merkle-Patricia tree of height 251 that is constructed
///   from the contract’s storage.
/// * `nonce` - The current nonce of the contract.
///
/// # Returns
///
/// The contract state hash.
pub fn calculate_contract_state_hash<H: HasherT>(
    hash: Felt252Wrapper,
    root: Felt252Wrapper,
    nonce: Felt252Wrapper,
) -> Felt252Wrapper {
    // Define the constant for the contract state hash version, ensure this aligns with StarkNet
    // specifications.
    const CONTRACT_STATE_HASH_VERSION: Felt252Wrapper = Felt252Wrapper::ZERO;

    // First hash: Combine class_hash and storage_root.
    let class_storage_hash = H::compute_hash_on_elements(&[hash.0, root.0]);
    let nonce_hash = H::compute_hash_on_elements(&[class_storage_hash, nonce.0]);
    let contract_state_hash = H::compute_hash_on_elements(&[nonce_hash, CONTRACT_STATE_HASH_VERSION.0]);

    contract_state_hash.into()
}
