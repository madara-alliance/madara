use std::sync::Arc;

use bitvec::vec::BitVec;
use bonsai_trie::id::BasicIdBuilder;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use crossbeam_skiplist::SkipMap;
use lazy_static::lazy_static;
use mc_db::bonsai_db::BonsaiDb;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use sp_runtime::traits::Block as BlockT;
use starknet_api::transaction::Event;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;
use tokio::task::JoinSet;

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
pub(crate) async fn event_commitment<B, H>(
    events: &[Event],
    backend: &Arc<BonsaiDb<B>>,
) -> Result<Felt252Wrapper, String>
where
    B: BlockT,
    H: HasherT,
{
    if events.is_empty() {
        return Ok(Felt252Wrapper::ZERO);
    }

    let config = BonsaiStorageConfig::default();
    let mut bonsai_storage =
        BonsaiStorage::<_, _, Pedersen>::new(backend.as_ref(), config).expect("Failed to create bonsai storage");

    let mut id_builder = BasicIdBuilder::new();

    let zero = id_builder.new_id();
    bonsai_storage.commit(zero).expect("Failed to commit to bonsai storage");

    // event hashes are calculated in parallel
    let mut set = JoinSet::new();
    for (i, event) in events.iter().cloned().enumerate() {
        let arc_event = Arc::new(event);
        set.spawn(async move { (i, get_hash::<H>(&Arc::clone(&arc_event))) });
    }

    // resulting hashes are waited for and added to the Bonsai Trie db
    while let Some(res) = set.join_next().await {
        let (i, event_hash) = res.map_err(|e| format!("Failed to compute event hash: {e}"))?;
        let key = BitVec::from_vec(i.to_be_bytes().to_vec());
        let value = Felt::from(Felt252Wrapper::from(event_hash));
        bonsai_storage
            .insert(key.as_bitslice(), &value)
            .map_err(|_| format!("Failed to insert into bonsai storage"))?;
    }

    // Note that committing changes still has the greatest performance hit
    // as this is where the root hash is calculated. Due to the Merkle structure
    // of Bonsai Tries, this results in a trie size that grows very rapidly with
    // each new insertion. It seems that the only vector of optimization here
    // would be to optimize the tree traversal and hash computation.
    let id = id_builder.new_id();
    bonsai_storage.commit(id).map_err(|_| format!("Failed to commit to bonsai storage"))?;

    // restores the Bonsai Trie to it's previous state
    let root_hash = bonsai_storage.root_hash().map_err(|_| format!("Failed to get root hash"))?;
    bonsai_storage.revert_to(zero).unwrap();

    Ok(Felt252Wrapper::from(root_hash))
}

// Event hashes are cached to avoid re-computing hashes for duplicate events.
// Note that this does not seem to have a huge impact on performance,
// so might be removed in the future if the memory footprint becomes an issue.
lazy_static! {
    static ref EVENT_HASHES: SkipMap<Event, FieldElement> = SkipMap::new();
}

fn get_hash<H>(event: &Event) -> FieldElement
where
    H: HasherT,
{
    match EVENT_HASHES.get(event) {
        Some(entry) => entry.value().clone(),
        None => store_hash::<H>(event),
    }
}

fn store_hash<H>(event: &Event) -> FieldElement
where
    H: HasherT,
{
    let event_hash = calculate_event_hash::<H>(event);
    EVENT_HASHES.insert(event.clone(), event_hash);

    event_hash
}
