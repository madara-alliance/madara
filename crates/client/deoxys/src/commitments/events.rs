use std::sync::Arc;

use anyhow::Ok;
use bitvec::bits;
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bonsai_trie::bonsai_database::DatabaseKey;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiDatabase, BonsaiStorage, BonsaiStorageConfig, BonsaiTrieHash, Membership, ProofNode};
use mc_db::bonsai_db::BonsaiDb;
use mc_db::{Backend, BonsaiDbError};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::Transaction;
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
use sp_runtime::traits::Block as BlockT;
use starknet_api::transaction::Event;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

/// Calculate the hash of an event.
///
/// See the [documentation](https://docs.starknet.io/documentation/architecture_and_concepts/Events/starknet-events/#event_hash)
/// for details.
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

/// Calculate event commitment hash value.
///
/// The event commitment is the root of the Patricia Merkle tree with height 64
/// constructed by adding the event hash
/// (see https://docs.starknet.io/documentation/architecture_and_concepts/Events/starknet-events/#event_hash)
/// to the tree and computing the root hash.
///
/// # Arguments
///
/// * `events` - The events to calculate the commitment from.
///
/// # Returns
///
/// The merkle root of the merkle tree built from the events.
pub(crate) fn calculate_event_commitment<B, H>(
    events: &[Event],
    backend: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, anyhow::Error>
where
    B: BlockT,
    H: HasherT,
{
    if events.len() > 0 {
        let config = BonsaiStorageConfig::default();
        let bonsai_db = backend.as_ref();
        let mut bonsai_storage =
            BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");
        
        let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
        println!("Root hash: {}", root_hash);
        let mut id_builder = BasicIdBuilder::new();

        let zero = id_builder.new_id();
        bonsai_storage.commit(zero).expect("Failed to commit to bonsai storage");

        for (i, event) in events.iter().enumerate() {
            let event_hash = calculate_event_hash::<H>(event);
            let key = BitVec::from_vec(i.to_be_bytes().to_vec());
            let value = Felt::from(Felt252Wrapper::from(event_hash));
            bonsai_storage.insert(key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
        }

        let id = id_builder.new_id();
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");

        let root_hash = bonsai_storage.root_hash().expect("Failed to get root hash");
        bonsai_storage.revert_to(zero).unwrap();
        println!("Root hash 2: {:?}", Felt252Wrapper::from(root_hash));

        Ok(Felt252Wrapper::from(root_hash))
    } else {
        Ok(Felt252Wrapper::ZERO)
    } 
}
