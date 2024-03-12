use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;

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
    let contract_class_hash_version = Felt252Wrapper::try_from("CONTRACT_CLASS_LEAF_V0".as_bytes()).unwrap();

    let hash = H::hash_elements(contract_class_hash_version.0, compiled_class_hash.0);

    hash.into()
}
