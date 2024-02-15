use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::Transaction;
use starknet_api::transaction::Event;

use super::events::memory_event_commitment;
use super::transactions::memory_transaction_commitment;

/// Calculate the transaction commitment, the event commitment and the event count.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
///
/// # Returns
///
/// The transaction commitment, the event commitment and the event count.
pub fn calculate_commitments<H: HasherT>(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> (Felt252Wrapper, Felt252Wrapper) {
    (
        memory_transaction_commitment::<H>(transactions, chain_id, block_number)
            .expect("Failed to calculate transaction commitment"),
        memory_event_commitment::<H>(events).expect("Failed to calculate event commitment"),
    )
}

/// Calculate state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251.
///
/// # Arguments
///
/// * `contracts_trie_root` - The root of the contracts trie.
/// * `classes_trie_root` - The root of the classes trie.
///
/// # Returns
///
/// The state commitment as a `StateCommitment`.
pub fn calculate_state_commitment<H: HasherT>(
    contracts_trie_root: Felt252Wrapper,
    classes_trie_root: Felt252Wrapper,
) -> Felt252Wrapper
where
    H: HasherT,
{
    let starknet_state_prefix = Felt252Wrapper::try_from("STARKNET_STATE_V0".as_bytes()).unwrap();

    let state_commitment_hash =
        H::compute_hash_on_elements(&[starknet_state_prefix.0, contracts_trie_root.0, classes_trie_root.0]);

    state_commitment_hash.into()
}
