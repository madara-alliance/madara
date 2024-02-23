use std::sync::Arc;

use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::Transaction;
use sp_runtime::traits::Block as BlockT;
use starknet_api::transaction::Event;
use tokio::join;

use super::events::event_commitment;
use super::transactions::transaction_commitment;

/// Calculate the transaction commitment, the event commitment and the event count.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
///
/// # Returns
///
/// The transaction commitment, the event commitment and the event count.
pub async fn calculate_commitments<B: BlockT, H: HasherT>(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt252Wrapper,
    block_number: u64,
    backend: Arc<mc_db::Backend<B>>,
) -> (Felt252Wrapper, Felt252Wrapper) {
    let bonsai = backend.bonsai();

    let (commitment_tx, commitment_event) = join!(
        transaction_commitment::<B, H>(transactions, chain_id, block_number, bonsai),
        event_commitment::<B, H>(events, bonsai)
    );

    (
        commitment_tx.expect("Failed to calculate transaction commitment"),
        commitment_event.expect("Failed to calculate event commitment"),
    )
}

// /// Calculate the transaction commitment, the event commitment and the event count.
// ///
// /// # Arguments
// ///
// /// * `transactions` - The transactions of the block
// ///
// /// # Returns
// ///
// /// The transaction commitment, the event commitment and the event count.
// pub fn calculate_state_commitments<B: BlockT, H: HasherT>(
//     transactions: &[Transaction],
//     events: &[Event],
//     chain_id: Felt252Wrapper,
//     block_number: u64,
//     backend: Arc<mc_db::Backend<B>>,
// ) -> Felt252Wrapper { state_commitment::<B, H>(transactions, chain_id, block_number,
//   &backend.bonsai().clone()) .expect("Failed to calculate transaction commitment")
// }
