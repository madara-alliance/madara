use mc_db::MadaraBackend;
use mp_block::PendingFullBlock;
use mp_class::ConvertedClass;
use starknet_core::types::Felt;

/// Returns the block_hash of the saved block.
#[tracing::instrument(skip(backend, block), fields(module = "BlockProductionTask"))]
pub async fn close_and_save_block(
    backend: &MadaraBackend,
    block: PendingFullBlock,
    classes: Vec<ConvertedClass>,
    block_number: u64,
) -> anyhow::Result<Felt> {
    let block_hash = backend
        .add_full_block_with_classes(block, block_number, &classes, /* pre_v0_13_2_hash_override */ true)
        .await?;

    Ok(block_hash)
}
