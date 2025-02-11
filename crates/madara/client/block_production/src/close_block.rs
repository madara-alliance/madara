use mc_db::{MadaraBackend, MadaraStorageError};
use mp_block::{
    commitments::CommitmentComputationContext, MadaraPendingBlock, PendingFullBlock, TransactionWithReceipt,
};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use starknet_core::types::Felt;
use std::iter;

/// Returns the block_hash of the saved block.
#[tracing::instrument(skip(backend, state_diff, declared_classes), fields(module = "BlockProductionTask"))]
pub fn close_and_save_block(
    backend: &MadaraBackend,
    block: MadaraPendingBlock,
    state_diff: StateDiff,
    block_number: u64,
    declared_classes: Vec<ConvertedClass>,
) -> Result<Felt, MadaraStorageError> {
    let block = PendingFullBlock {
        header: block.info.header,
        state_diff,
        events: block
            .inner
            .receipts
            .iter()
            .flat_map(|receipt| {
                receipt
                    .events()
                    .iter()
                    .cloned()
                    .map(|event| EventWithTransactionHash { transaction_hash: receipt.transaction_hash(), event })
            })
            .collect(),
        transactions: block
            .inner
            .transactions
            .into_iter()
            .zip(block.inner.receipts)
            .map(|(transaction, receipt)| TransactionWithReceipt { receipt, transaction })
            .collect(),
    };

    // Apply state, compute state root
    let new_global_state_root = backend.apply_state(block_number, iter::once(&block.state_diff))?;

    // Compute the block merkle commitments.
    let block = block.close_block(
        &CommitmentComputationContext {
            protocol_version: backend.chain_config().latest_protocol_version,
            chain_id: backend.chain_config().chain_id.to_felt(),
        },
        block_number,
        new_global_state_root,
        true,
    );
    let block_hash = block.block_hash;

    backend.store_full_block(block)?;
    backend.class_db_store_block(block_number, &declared_classes)?;

    Ok(block_hash)
}
