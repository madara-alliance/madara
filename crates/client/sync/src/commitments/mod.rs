mod classes;
mod contracts;
mod events;
mod transactions;

use blockifier::state::cached_state::CommitmentStateDiff;
use classes::class_trie_root;
use contracts::contract_trie_root;
use dc_db::DeoxysBackend;
use dp_convert::ToStarkFelt;
use dp_receipt::Event;
use events::memory_event_commitment;
use indexmap::IndexMap;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::Transaction;
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateUpdate,
    StorageEntry,
};
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Poseidon, StarkHash};
use transactions::memory_transaction_commitment;

/// "STARKNET_STATE_V0"
const STARKNET_STATE_PREFIX: Felt =
    Felt::from_raw([329108408257827203, 18446744073709548949, 8635008616843941494, 17245362975199821124]);

/// Calculate the transaction and event commitment.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
/// * `events` - The events of the block
/// * `chain_id` - The current chain id
/// * `block_number` - The current block number
///
/// # Returns
///
/// The transaction and the event commitment as `Felt`.
pub fn calculate_tx_and_event_commitments(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt,
    block_number: u64,
) -> ((Felt, Vec<Felt>), Felt) {
    let (commitment_tx, commitment_event) = rayon::join(
        || memory_transaction_commitment(transactions, chain_id, block_number),
        || memory_event_commitment(events),
    );
    (
        commitment_tx.expect("Failed to calculate transaction commitment"),
        commitment_event.expect("Failed to calculate event commitment"),
    )
}

/// Aggregates all the changes from last state update in a way that is easy to access
/// when computing the state root
///
/// * `state_update`: The last state update fetched from the sequencer
pub fn build_commitment_state_diff(state_update: &StateUpdate) -> CommitmentStateDiff {
    let mut commitment_state_diff = CommitmentStateDiff {
        address_to_class_hash: IndexMap::new(),
        address_to_nonce: IndexMap::new(),
        storage_updates: IndexMap::new(),
        class_hash_to_compiled_class_hash: IndexMap::new(),
    };

    for DeployedContractItem { address, class_hash } in state_update.state_diff.deployed_contracts.iter() {
        let address: ContractAddress = address.to_stark_felt().try_into().unwrap();
        let class_hash = if *address.0.key() == StarkFelt::ZERO {
            // System contracts doesnt have class hashes
            ClassHash(StarkFelt::ZERO)
        } else {
            ClassHash(class_hash.to_stark_felt())
        };
        commitment_state_diff.address_to_class_hash.insert(address, class_hash);
    }

    for ReplacedClassItem { contract_address, class_hash } in state_update.state_diff.replaced_classes.iter() {
        let address = contract_address.to_stark_felt().try_into().unwrap();
        let class_hash = ClassHash(class_hash.to_stark_felt());
        commitment_state_diff.address_to_class_hash.insert(address, class_hash);
    }

    for DeclaredClassItem { class_hash, compiled_class_hash } in state_update.state_diff.declared_classes.iter() {
        let class_hash = ClassHash(class_hash.to_stark_felt());
        let compiled_class_hash = CompiledClassHash(compiled_class_hash.to_stark_felt());
        commitment_state_diff.class_hash_to_compiled_class_hash.insert(class_hash, compiled_class_hash);
    }

    for NonceUpdate { contract_address, nonce } in state_update.state_diff.nonces.iter() {
        let contract_address = contract_address.to_stark_felt().try_into().unwrap();
        let nonce_value = Nonce(nonce.to_stark_felt());
        commitment_state_diff.address_to_nonce.insert(contract_address, nonce_value);
    }

    for ContractStorageDiffItem { address, storage_entries } in state_update.state_diff.storage_diffs.iter() {
        let contract_address = address.to_stark_felt().try_into().unwrap();
        let mut storage_map = IndexMap::new();
        for StorageEntry { key, value } in storage_entries.iter() {
            let key = key.to_stark_felt().try_into().unwrap();
            let value = value.to_stark_felt();
            storage_map.insert(key, value);
        }
        commitment_state_diff.storage_updates.insert(contract_address, storage_map);
    }

    commitment_state_diff
}

/// Calculate state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `contracts_trie_root` - The root of the contracts trie.
/// * `classes_trie_root` - The root of the classes trie.
///
/// # Returns
///
/// The state commitment as a `Felt`.
pub fn calculate_state_root(contracts_trie_root: Felt, classes_trie_root: Felt) -> Felt {
    if classes_trie_root == Felt::ZERO {
        contracts_trie_root
    } else {
        Poseidon::hash_array(&[STARKNET_STATE_PREFIX, contracts_trie_root, classes_trie_root])
    }
}

/// Update the state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `CommitmentStateDiff` - The commitment state diff inducing unprocessed state changes.
/// * `BonsaiDb` - The database responsible for storing computing the state tries.
///
///
/// The updated state root as a `Felt`.
pub fn csd_calculate_state_root(backend: &DeoxysBackend, csd: CommitmentStateDiff, block_number: u64) -> Felt {
    // Update contract and its storage tries
    let (contract_trie_root, class_trie_root) = rayon::join(
        || contract_trie_root(backend, &csd, block_number).expect("Failed to compute contract root"),
        || class_trie_root(backend, &csd, block_number).expect("Failed to compute class root"),
    );
    calculate_state_root(contract_trie_root, class_trie_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starknet_state_version() {
        assert_eq!(STARKNET_STATE_PREFIX, Felt::from_bytes_be_slice("STARKNET_STATE_V0".as_bytes()));
    }
}
