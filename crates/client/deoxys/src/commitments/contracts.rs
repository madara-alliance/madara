use std::sync::Arc;

use bonsai_trie::id::BasicIdBuilder;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbError;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use sp_runtime::traits::Block as BlockT;
use starknet_types_core::hash::Pedersen;

pub struct ContractLeafParams {
    pub hash: Felt252Wrapper,
    pub root: Felt252Wrapper,
    pub nonce: Felt252Wrapper,
}

/// Calculates the storage root.
///
/// `storage_root` is the root of another Merkle-Patricia trie of height 251 that is constructed
/// from the contract’s storage.
///
/// # Arguments
///
///
/// # Returns
///
/// The storage root hash.
pub fn storage_root<H: HasherT>() -> Felt252Wrapper {
    Felt252Wrapper::default()
}

/// Calculates the contract state hash from its preimage.
///
/// # Arguments
///
/// * `hash` - The hash of the contract definition.
/// * `root` - The root of root of another Merkle-Patricia trie of height 251 that is constructed
///   from the contract’s storage.
/// * `nonce` - The current nonce of the contract.
///
/// # Returns
///
/// The contract state leaf hash.
pub fn calculate_contract_state_leaf_hash<H: HasherT>(contract_leaf_params: ContractLeafParams) -> Felt252Wrapper {
    // Define the constant for the contract state hash version
    const CONTRACT_STATE_HASH_VERSION: Felt252Wrapper = Felt252Wrapper::ZERO;

    let class_storage_hash = H::compute_hash_on_elements(&[contract_leaf_params.hash.0, contract_leaf_params.root.0]);
    let nonce_hash = H::compute_hash_on_elements(&[class_storage_hash, contract_leaf_params.nonce.0]);
    let contract_state_hash =
        H::compute_hash_on_elements(&[nonce_hash, CONTRACT_STATE_HASH_VERSION.0]);

    contract_state_hash.into()
}

/// Calculates the contract trie root.
///
/// # Arguments
///
/// # Returns
///
/// The contract root hash.
pub fn calculate_contract_trie_root<B, H>(
    contract_hash: Felt252Wrapper,
    contract_leaf_params: ContractLeafParams,
    backend: &Arc<BonsaiDb<B>>,
) -> Felt252Wrapper
where
    B: BlockT,
    H: HasherT,
{
    // let config = BonsaiStorageConfig::default();
    // let bonsai_db = backend.as_ref();
    // let mut bonsai_storage =
    //     BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    // let contract_state_hash = calculate_contract_state_leaf_hash(contract_leaf_params);

    // bonsai_storage
    //     .insert(contract_hash.as_bitslice(), &contract_state_hash.into())
    //     .expect("Failed to insert into bonsai storage");

    // let id_builder = BasicIdBuilder::new();
    // let id = id_builder.new_id();
    // bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    // let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    // Ok(Felt252Wrapper::from(root_hash))
    Felt252Wrapper::default()
}
