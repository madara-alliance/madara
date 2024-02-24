use std::sync::Arc;

use anyhow::Ok;
use bitvec::vec::BitVec;
use bonsai_trie::databases::HashMapDb;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mc_db::bonsai_db::BonsaiDb;
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::HasherT;
use sp_runtime::traits::Block as BlockT;
use starknet_api::transaction::Event;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

/// Calculate the hash of the event.
///
/// # Arguments
///
/// * `event` - The event we want to calculate the hash of.
///
/// # Returns
///
/// The event hash as `FieldElement`.
pub fn calculate_event_hash<H: HasherT>(event: &Event) -> FieldElement {
    let keys_hash = H::compute_hash_on_elements(
        &event
            .content
            .keys
            .iter()
            .map(|key| FieldElement::from(Felt252Wrapper::from(key.0)))
            .collect::<Vec<FieldElement>>(),
    );
    let data_hash = H::compute_hash_on_elements(
        &event
            .content
            .data
            .0
            .iter()
            .map(|data| FieldElement::from(Felt252Wrapper::from(*data)))
            .collect::<Vec<FieldElement>>(),
    );
    let from_address = FieldElement::from(Felt252Wrapper::from(event.from_address.0.0));
    H::compute_hash_on_elements(&[from_address, keys_hash, data_hash])
}

/// Calculate the event commitment in storage using BonsaiDb (which is less efficient for this
/// usecase).
///
/// # Arguments
///
/// * `events` - The events of the block
/// * `bonsai_db` - The bonsai database responsible to compute the tries
///
/// # Returns
///
/// The event commitment as `Felt252Wrapper`.
pub fn event_commitment<B: BlockT>(
    events: &[Event],
    bonsai_db: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, anyhow::Error> {
    if events.len() > 0 {
        let config = BonsaiStorageConfig::default();
        let bonsai_db = bonsai_db.as_ref();
        let mut bonsai_storage =
            BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

        let mut id_builder = BasicIdBuilder::new();

        let zero = id_builder.new_id();
        bonsai_storage.commit(zero).expect("Failed to commit to bonsai storage");

        for (i, event) in events.iter().enumerate() {
            let event_hash = calculate_event_hash::<PedersenHasher>(event);
            let key = BitVec::from_vec(i.to_be_bytes().to_vec());
            let value = Felt::from(Felt252Wrapper::from(event_hash));
            bonsai_storage.insert(key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
        }

        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

        let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
        bonsai_storage.revert_to(zero).unwrap();

        Ok(Felt252Wrapper::from(root_hash))
    } else {
        Ok(Felt252Wrapper::ZERO)
    }
}

/// Calculate the event commitment in memory using HashMapDb (which is more efficient for this
/// usecase).
///
/// # Arguments
///
/// * `events` - The events of the block
///
/// # Returns
///
/// The event commitment as `Felt252Wrapper`.
pub fn memory_event_commitment(events: &[Event]) -> Result<Felt252Wrapper, anyhow::Error> {
    if !events.is_empty() {
        let config = BonsaiStorageConfig::default();
        let bonsai_db = HashMapDb::<BasicId>::default();
        let mut bonsai_storage =
            BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

        for (i, event) in events.iter().enumerate() {
            let event_hash = calculate_event_hash::<PedersenHasher>(event);
            let key = BitVec::from_vec(i.to_be_bytes().to_vec());
            let value = Felt::from(Felt252Wrapper::from(event_hash));
            bonsai_storage.insert(key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
        }

        let mut id_builder = BasicIdBuilder::new();
        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

        let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
        Ok(Felt252Wrapper::from(root_hash))
    } else {
        Ok(Felt252Wrapper::ZERO)
    }
}
