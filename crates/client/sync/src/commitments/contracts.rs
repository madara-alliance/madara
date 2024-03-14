use std::sync::{Arc, Mutex, MutexGuard};

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::databases::HashMapDb;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig, BonsaiStorageError};
use indexmap::IndexMap;
use mc_db::bonsai_db::BonsaiDb;
use mc_db::BonsaiDbError;
use mc_storage::OverrideHandle;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_storage::StarknetStorageSchemaVersion::Undefined;
use sp_core::hexdisplay::AsBytesRef;
use sp_core::H256;
use sp_runtime::generic::{Block, Header};
use sp_runtime::traits::{BlakeTwo256, Block as BlockT};
use sp_runtime::OpaqueExtrinsic;
use starknet_api::api_core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_types_core::hash::Pedersen;

#[derive(Debug)]
pub struct ContractLeafParams {
    pub class_hash: Felt252Wrapper,
    pub storage_root: Felt252Wrapper,
    pub nonce: Felt252Wrapper,
}

/// Calculates the storage root in memory recomupting all the storage changes for a specific
/// contract. NOTE: in the future this function should be persistent, replaced with a more efficient
/// way computing only changes.
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
    storage_updates: &IndexMap<StorageKey, StarkFelt>,
    bonsai_contract_storage: &Arc<Mutex<BonsaiStorage<BasicId, BonsaiDb<B>, Pedersen>>>,
) {
    let mut bonsai_storage = bonsai_contract_storage.lock().unwrap();
    let identifier = contract_address.0.0.0.as_bytes_ref();
    bonsai_storage.init_tree(&identifier).expect("Failed to init tree");

    // Insert new storage changes
    storage_updates.into_iter().map(|(key, value)| convert_storage((*key, *value))).for_each(|(key, value)| {
        if value != Felt252Wrapper::ZERO {
            bonsai_storage.insert(&identifier, &key, &value.into()).expect("Failed to insert storage update into trie");
        }
    });
}

fn convert_storage(storage: (StorageKey, StarkFelt)) -> (BitVec<u8, Msb0>, Felt252Wrapper) {
    let (storage_key, storage_value) = storage;
    let key = Felt252Wrapper::from(storage_key.0.0).0.to_bytes_be().view_bits()[5..].to_owned();
    let value = Felt252Wrapper::from(storage_value);

    (key, value)
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
