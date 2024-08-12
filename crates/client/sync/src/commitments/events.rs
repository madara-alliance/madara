use mp_block::StarknetVersion;
use mp_receipt::Event;
use rayon::prelude::*;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon};

use super::compute_root;

/// Calculate the event commitment in memory using HashMapDb (which is more efficient for this
/// usecase).
///
/// # Arguments
///
/// * `events` - The events of the block
///
/// # Returns
///
/// The event commitment as `Felt`.
pub fn memory_event_commitment(events_with_tx_hash: &[(Felt, Event)], starknet_version: StarknetVersion) -> Felt {
    if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
        memory_event_commitment_pedersen(events_with_tx_hash)
    } else {
        memory_event_commitment_poseidon(events_with_tx_hash)
    }
}

fn memory_event_commitment_pedersen(events_with_tx_hash: &[(Felt, Event)]) -> Felt {
    if events_with_tx_hash.is_empty() {
        return Felt::ZERO;
    }

    // event hashes are computed in parallel
    let events_hash =
        events_with_tx_hash.into_par_iter().map(|(_, event)| event.compute_hash_pedersen()).collect::<Vec<_>>();

    // once event hashes have finished computing, they are inserted into the local Bonsai db
    compute_root::<Pedersen>(&events_hash)
}

fn memory_event_commitment_poseidon(events_with_tx_hash: &[(Felt, Event)]) -> Felt {
    if events_with_tx_hash.is_empty() {
        return Felt::ZERO;
    }

    // event hashes are computed in parallel
    let events_hash =
        events_with_tx_hash.into_par_iter().map(|(hash, event)| event.compute_hash_poseidon(hash)).collect::<Vec<_>>();

    // once event hashes have finished computing, they are inserted into the local Bonsai db
    compute_root::<Poseidon>(&events_hash)
}
