use std::sync::Arc;

use bitvec::prelude::BitVec;
use blockifier::state::cached_state::CommitmentStateDiff;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbError;
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::HasherT;
use sp_runtime::traits::Block as BlockT;
use starknet_types_core::hash::Pedersen;

pub struct ContractLeafParams {
    pub class_hash: Felt252Wrapper,
    pub storage_root: Felt252Wrapper,
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
pub fn update_storage_trie<B: BlockT>(
    commitment_state_diff: CommitmentStateDiff,
    bonsai_db: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let mut bonsai_storage: BonsaiStorage<_, _, Pedersen> = BonsaiStorage::new(bonsai_db.as_ref(), config)
        .expect("Failed to create bonsai storage");

    for (contract_address, updates) in &commitment_state_diff.storage_updates {
        for (storage_key, storage_value) in updates {
            let key = BitVec::from_vec(Felt252Wrapper::from(storage_key.0.0).0.to_bytes_be()[..31].to_vec());
            let value = Felt252Wrapper::from(*storage_value);

            bonsai_storage.insert(key.as_bitslice(), &value.into())
                .expect("Failed to insert storage update into trie");
        }
    }

    let mut id_builder = BasicIdBuilder::new();
    let id = id_builder.new_id();
    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    // After all updates are inserted, compute the root hash of the trie
    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}

pub fn get_storage_trie_root<B: BlockT>(bonsai_db: &Arc<BonsaiDb<B>>) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();
    let bonsai_storage: BonsaiStorage<BasicId, &BonsaiDb<B>, Pedersen> =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    // set column context to BONSAI_STORAGE column id
    // println!("Current column is: {:?}", bonsai_db.current_column);

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
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

    let contract_state_hash = H::compute_hash_on_elements(&[
        contract_leaf_params.class_hash.0,
        contract_leaf_params.storage_root.0,
        contract_leaf_params.nonce.0,
        CONTRACT_STATE_HASH_VERSION.0,
    ]);

    contract_state_hash.into()
}

pub fn update_contract_trie<B: BlockT>(
    contract_hash: Felt252Wrapper,
    contract_leaf_params: ContractLeafParams,
    bonsai_db: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();

    let mut bonsai_storage = BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect(
        "Failed to create bonsai
    storage",
    );

    // println!("Current column is: {:?}", bonsai_db.current_column);
    let class_commitment_leaf_hash = calculate_contract_state_leaf_hash::<PedersenHasher>(contract_leaf_params);
    let key = BitVec::from_vec(contract_hash.0.to_bytes_be()[..31].to_vec());
    bonsai_storage
        .insert(key.as_bitslice(), &class_commitment_leaf_hash.into())
        .expect("Failed to insert into bonsai storage");

    let mut id_builder = BasicIdBuilder::new();
    let id = id_builder.new_id();
    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}

pub fn get_contract_trie_root<B: BlockT>(bonsai_db: &Arc<BonsaiDb<B>>) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();
    let mut bonsai_storage: BonsaiStorage<BasicId, &BonsaiDb<B>, Pedersen> =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    // println!("Current column is: {:?}", bonsai_db.current_column);

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}
