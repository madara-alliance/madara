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
use starknet_api::api_core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

#[derive(Debug)]
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
    contract_address: &ContractAddress,
    csd: &Arc<CommitmentStateDiff>,
    bonsai_db: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();
    let mut bonsai_storage: BonsaiStorage<BasicId, &BonsaiDb<B>, Pedersen> =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    if let Some(updates) = csd.storage_updates.get(contract_address) {
        for (storage_key, storage_value) in updates {
            let key = BitVec::from_vec(Felt252Wrapper::from(storage_key.0.0).0.to_bytes_be()[..31].to_vec());
            let value = Felt252Wrapper::from(*storage_value);
            bonsai_storage.insert(key.as_bitslice(), &value.into()).expect("Failed to insert storage update into trie");
        }
    }

    let mut id_builder = BasicIdBuilder::new();
    let id = id_builder.new_id();
    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");

    Ok(Felt252Wrapper::from(root_hash))
}

/// Get the storage trie root hash of the actual contract state.
///
/// # Arguments
///
/// * `bonsai_db` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The contract state root hash as a `Felt252Wrapper` or a `BonsaiDbError`.
pub fn get_storage_trie_root<B: BlockT>(bonsai_db: &Arc<BonsaiDb<B>>) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();
    let bonsai_storage: BonsaiStorage<BasicId, &BonsaiDb<B>, Pedersen> =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}

/// Calculates the contract state hash.
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

    let contract_state_hash = H::hash_elements(contract_leaf_params.class_hash.0, contract_leaf_params.storage_root.0);
    let contract_state_hash = H::hash_elements(contract_state_hash, contract_leaf_params.nonce.0);
    let contract_state_hash = H::hash_elements(contract_state_hash, CONTRACT_STATE_HASH_VERSION.0);

    contract_state_hash.into()
}

/// Update the contract trie with the new contract state.
///
/// # Arguments
///
/// * `contract_hash` - The hash of the contract.
/// * `contract_leaf_params` - A struct containing the class hash, storage root and nonce.
/// * `bonsai_db` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The contract state root hash as a `Felt252Wrapper` or a `BonsaiDbError`.
pub fn update_contract_trie<B: BlockT>(
    contract_hash: Felt252Wrapper,
    contract_leaf_params: ContractLeafParams,
    bonsai_db: &Arc<BonsaiDb<B>>,
) -> anyhow::Result<Felt252Wrapper> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();
    let mut bonsai_storage =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

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

/// Get the actual contract trie hash.
///
/// # Arguments
///
/// * `bonsai_db` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The contract state root hash as a `Felt252Wrapper`or a `BonsaiDbError`.
pub fn get_contract_trie_root<B: BlockT>(bonsai_db: &Arc<BonsaiDb<B>>) -> Result<Felt252Wrapper, BonsaiDbError> {
    let config = BonsaiStorageConfig::default();
    let bonsai_db = bonsai_db.as_ref();
    let bonsai_storage: BonsaiStorage<BasicId, &BonsaiDb<B>, Pedersen> =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
    Ok(Felt252Wrapper::from(root_hash))
}

#[cfg(test)]
mod tests {
    use mp_felt::Felt252Wrapper;
    use mp_hashers::pedersen::PedersenHasher;

    use super::calculate_contract_state_leaf_hash;

    #[test]
    fn test_contract_leaf_hash() {
        let contract_leaf_params = super::ContractLeafParams {
            class_hash: Felt252Wrapper::from_hex_be(
                "0x2ff4903e17f87b298ded00c44bfeb22874c5f73be2ced8f1d9d9556fb509779",
            )
            .unwrap(),
            storage_root: Felt252Wrapper::from_hex_be(
                "0x4fb440e8ca9b74fc12a22ebffe0bc0658206337897226117b985434c239c028",
            )
            .unwrap(),
            nonce: Felt252Wrapper::ZERO,
        };

        let expected =
            Felt252Wrapper::from_hex_be("0x7161b591c893836263a64f2a7e0d829c92f6956148a60ce5e99a3f55c7973f3").unwrap();

        let result = calculate_contract_state_leaf_hash::<PedersenHasher>(contract_leaf_params);

        assert_eq!(result, expected);
    }
}
