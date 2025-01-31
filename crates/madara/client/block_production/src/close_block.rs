use mc_block_import::{
    BlockImportError, BlockImportResult, BlockImporter, BlockValidationContext, UnverifiedFullBlock, UnverifiedHeader,
};
use mp_block::{header::PendingHeader, MadaraPendingBlock, MadaraPendingBlockInfo};
use mp_class::ConvertedClass;
use mp_state_update::StateDiff;
use starknet_api::core::ChainId;

/// Close the block (convert from pending to closed), and store to db. This is delegated to the block import module.
#[tracing::instrument(skip(importer, state_diff, declared_classes), fields(module = "BlockProductionTask"))]
pub async fn close_block(
    importer: &BlockImporter,
    block: MadaraPendingBlock,
    state_diff: &StateDiff,
    chain_id: ChainId,
    block_number: u64,
    declared_classes: Vec<ConvertedClass>,
) -> Result<BlockImportResult, BlockImportError> {
    let validation = BlockValidationContext::new(chain_id).trust_transaction_hashes(true);

    let MadaraPendingBlock { info, inner } = block;
    let MadaraPendingBlockInfo { header, tx_hashes: _tx_hashes } = info;

    // Header
    let PendingHeader {
        parent_block_hash,
        sequencer_address,
        block_timestamp,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
    } = header;

    let block = importer
        .pre_validate(
            UnverifiedFullBlock {
                unverified_block_number: Some(block_number),
                header: UnverifiedHeader {
                    parent_block_hash: Some(parent_block_hash),
                    sequencer_address,
                    block_timestamp,
                    protocol_version,
                    l1_gas_price,
                    l1_da_mode,
                },
                state_diff: state_diff.clone(),
                transactions: inner.transactions,
                receipts: inner.receipts,
                trusted_converted_classes: declared_classes,
                commitments: Default::default(), // the block importer will compute the commitments for us
                ..Default::default()
            },
            validation.clone(),
        )
        .await?;

    importer.verify_apply(block, validation.clone()).await
}
