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
        // if value != Felt252Wrapper::ZERO {
            bonsai_storage.insert(&identifier, &key, &value.into()).expect("Failed to insert storage update into trie");
        // }
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
    use blockifier::execution::contract_address;
    use bonsai_trie::{databases::HashMapDb, id::{BasicId, BasicIdBuilder}, BonsaiStorage, BonsaiStorageConfig};
    use mp_felt::Felt252Wrapper;
    use mp_hashers::pedersen::PedersenHasher;
    use starknet_api::hash::StarkFelt;
    use starknet_types_core::{felt::Felt, hash::Pedersen};

    use crate::commitments::lib::key as keyer;

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

    #[test]
    fn test_insert_zero() {
        let config = BonsaiStorageConfig::default();
        let bonsai_db = HashMapDb::<BasicId>::default();
        let mut bonsai_storage =
            BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");
        let identifier = "0x056e4fed965fccd7fb01fcadd827470338f35ced62275328929d0d725b5707ba".as_bytes();

        // Insert Block 3 storage changes for contract `0x056e4fed965fccd7fb01fcadd827470338f35ced62275328929d0d725b5707ba`
        let block_3 = [
            ("0x5", "0x456"),
            ("0x378e096bb5e74b0f4ca78660a6b49b4a8035e571b024c018713c80b4b969735", "0x205d119502a165dae3830f627fa93fbdf5bfb13edd8f00e4c72621d0cda24"),
            ("0x41139bbf557d599fe8e96983251ecbfcb5bf4c4138c85946b0c4a6a68319f24", "0x7eec291f712520293664c7e3a8bb39ab00babf51cb0d9c1fb543147f37b485f"),
            ("0x77ae79c60260b3e48516a7da1aa173ac2765a5ced420f8ffd1539c394fbc03c", "0x6025343ab6a7ac36acde4eba3b6fc21f53d5302ee26e6f28e8de5a62bbfd847"),
            ("0x751901aac66fdc1f455c73022d02f1c085602cd0c9acda907cfca5418769e9c", "0x3f23078d48a4bf1d5f8ca0348f9efe9300834603625a379cae5d6d81100adef"),
            ("0x751901aac66fdc1f455c73022d02f1c085602cd0c9acda907cfca5418769e9d", "0xbd858a06904cadc3787ecbad97409606dcee50ea6fc30b94930bcf3d8843d5"),
        ];

        for (key_hex, value_hex) in block_3.iter() {
            let key: StarkFelt = Felt252Wrapper::from_hex_be(key_hex).unwrap().into();
            let value = Felt252Wrapper::from_hex_be(value_hex).unwrap();
            bonsai_storage.insert(&identifier, keyer(key).as_bitslice(), &value.into())
                .expect("Failed to insert storage update into trie");
        }

        let mut id_builder = BasicIdBuilder::new();
        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
        let root_hash = bonsai_storage.root_hash(&identifier).expect("Failed to get root hash");

        println!("Expected: 0x069064A05C14A9A2B4ED81C479C14D30872A9AE9CE2DEA8E4B4509542C2DCC1F\nFound: {:?}", Felt252Wrapper::from(root_hash));
        assert_eq!(Felt252Wrapper::from(root_hash), Felt252Wrapper::from_hex_be("0x069064A05C14A9A2B4ED81C479C14D30872A9AE9CE2DEA8E4B4509542C2DCC1F").unwrap());

        // Insert Block 4 storage changes for contract `0x056e4fed965fccd7fb01fcadd827470338f35ced62275328929d0d725b5707ba`
        let block_4 = [
            ("0x5", "0x0"), // Inserting key = 0x0
            ("0x4b81c1bca2d1b7e08535a5abe231b2e94399674db5e8f1d851fd8f4af4abd34", "0x7c7"),
            ("0x6f8cf54aaec1f42d5f3868d597fcd7393da888264dc5a6e93c7bd528b6d6fee", "0x7e5"),
            ("0x2a315469199dfde4b05906db8c33f6962916d462d8f1cf5252b748dfa174a20", "0xdae79d0308bb710af439eb36e82b405dc2bca23b351d08b4867d9525226e9d"),
            ("0x2d1ed96c7561dd8e5919657790ffba8473b80872fea3f7ef8279a7253dc3b33", "0x750387f4d66b0e9be1f2f330e8ad309733c46bb74e0be4df0a8c58fb4e89a25"),
            ("0x6a93bcb89fc1f31fa544377c7de6de1dd3e726e1951abc95c4984995e84ad0d", "0x7e5"),
            ("0x6b3b4780013c33cdca6799e8aa3ef922b64f5a2d356573b33693d81504deccf", "0x7c7"),
        ];

        for (key_hex, value_hex) in block_4.iter() {
            let key: StarkFelt = Felt252Wrapper::from_hex_be(key_hex).unwrap().into();
            let value = Felt252Wrapper::from_hex_be(value_hex).unwrap();
            bonsai_storage.insert(&identifier, keyer(key).as_bitslice(), &value.into())
                .expect("Failed to insert storage update into trie");
        }

        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
        let root_hash = bonsai_storage.root_hash(&identifier).expect("Failed to get root hash");

        println!("Expected: 0x0112998A41A3A2C720E758F82D184E4C39E9382620F12076B52C516D14622E57\nFound: {:?}", Felt252Wrapper::from(root_hash));
        assert_eq!(Felt252Wrapper::from(root_hash), Felt252Wrapper::from_hex_be("0x0112998A41A3A2C720E758F82D184E4C39E9382620F12076B52C516D14622E57").unwrap());

        // Insert Block 5 storage changes for contract `0x056e4fed965fccd7fb01fcadd827470338f35ced62275328929d0d725b5707ba`
        let block_5 = [
            ("0x5", "0x456"),
        ];

        for (key_hex, value_hex) in block_5.iter() {
            let key: StarkFelt = Felt252Wrapper::from_hex_be(key_hex).unwrap().into();
            let value = Felt252Wrapper::from_hex_be(value_hex).unwrap();
            bonsai_storage.insert(&identifier, keyer(key).as_bitslice(), &value.into())
                .expect("Failed to insert storage update into trie");
        }

        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
        let root_hash = bonsai_storage.root_hash(&identifier).expect("Failed to get root hash");

        println!("Expected: 0x072E79A6F71E3E63D7DE40EDF4322A22E64388D4D5BFE817C1271C78028B73BF\nFound: {:?}", Felt252Wrapper::from(root_hash));
        assert_eq!(Felt252Wrapper::from(root_hash), Felt252Wrapper::from_hex_be("0x072E79A6F71E3E63D7DE40EDF4322A22E64388D4D5BFE817C1271C78028B73BF").unwrap());

    }
}

