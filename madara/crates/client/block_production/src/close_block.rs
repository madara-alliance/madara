use mc_db::MadaraBackend;
use mp_block::{MadaraPendingBlock, PendingFullBlock, TransactionWithReceipt};
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use starknet_core::types::Felt;

/// Returns the block_hash of the saved block.
#[tracing::instrument(skip(backend, state_diff, declared_classes), fields(module = "BlockProductionTask"))]
pub async fn close_and_save_block(
    backend: &MadaraBackend,
    block: MadaraPendingBlock,
    state_diff: StateDiff,
    block_number: u64,
    declared_classes: Vec<ConvertedClass>,
) -> anyhow::Result<Felt> {
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

    let block_hash = backend
        .add_full_block_with_classes(block, block_number, &declared_classes, /* pre_v0_13_2_hash_override */ true)
        .await?;

    Ok(block_hash)
}
