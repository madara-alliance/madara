use bitvec::vec::BitVec;
use bonsai_trie::databases::HashMapDb;
use bonsai_trie::id::{BasicId, BasicIdBuilder};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::HasherT;
use starknet_api::transaction::Event;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;
use tokio::task::{spawn_blocking, JoinSet};

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
pub async fn memory_event_commitment(events: &[Event]) -> Result<Felt252Wrapper, String> {
    if events.is_empty() {
        return Ok(Felt252Wrapper::ZERO);
    }

    let config = BonsaiStorageConfig::default();
    let bonsai_db = HashMapDb::<BasicId>::default();
    let mut bonsai_storage =
        BonsaiStorage::<_, _, Pedersen>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    // event hashes are computed in parallel
    let mut task_set = JoinSet::new();
    events.iter().cloned().enumerate().for_each(|(i, event)| {
        task_set.spawn(async move { (i, calculate_event_hash::<PedersenHasher>(&event)) });
    });

    // once event hashes have finished computing, they are inserted into the local Bonsai db
    while let Some(res) = task_set.join_next().await {
        let (i, event_hash) = res.map_err(|e| format!("Failed to retrieve event hash: {e}"))?;
        let key = BitVec::from_vec(i.to_be_bytes().to_vec());
        let value = Felt::from(Felt252Wrapper::from(event_hash));
        bonsai_storage.insert(key.as_bitslice(), &value).expect("Failed to insert into bonsai storage");
    }

    // Note that committing changes still has the greatest performance hit
    // as this is where the root hash is calculated. Due to the Merkle structure
    // of Bonsai Tries, this results in a trie size that grows very rapidly with
    // each new insertion. It seems that the only vector of optimization here
    // would be to optimize the tree traversal and hash computation.
    let mut id_builder = BasicIdBuilder::new();
    let id = id_builder.new_id();

    // run in a blocking-safe thread to avoid starving the thread pool
    let root_hash = spawn_blocking(move || {
        bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
        bonsai_storage.root_hash().expect("Failed to get root hash")
    })
    .await
    .map_err(|e| format!("Failed to computed event root hash: {e}"))?;

    Ok(Felt252Wrapper::from(root_hash))
}
