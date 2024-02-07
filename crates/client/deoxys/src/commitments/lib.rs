use std::sync::Arc;

use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::Transaction;
use sp_runtime::traits::Block as BlockT;
use starknet_api::transaction::Event;

use super::events::calculate_event_commitment;
use super::transactions::calculate_transaction_commitment;
use mp_commitments::calculate_transaction_commitment as ctc;

/// Calculate the transaction commitment, the event commitment and the event count.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
///
/// # Returns
///
/// The transaction commitment, the event commitment and the event count.
pub fn calculate_commitments<B: BlockT, H: HasherT>(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt252Wrapper,
    block_number: u64,
    backend: Arc<mc_db::Backend<B>>,
) -> (Felt252Wrapper, Felt252Wrapper) {
    (
        ctc::<H>(transactions, chain_id, block_number),
        calculate_event_commitment::<B, H>(events, backend.bonsai())
            .expect("Failed to calculate event commitment"),
    )
}
