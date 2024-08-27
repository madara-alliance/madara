use dc_block_import::{BlockImportError, BlockImportResult, BlockImporter, UnverifiedFullBlock, UnverifiedHeader};
use dp_block::{header::PendingHeader, DeoxysPendingBlock, DeoxysPendingBlockInfo};
use dp_state_update::StateDiff;
use dp_validation::ValidationContext;
use starknet_api::core::ChainId;

/// Close the block (convert from pending to closed), and store to db. This is delegated to the block import module.
pub async fn close_block(
    importer: &BlockImporter,
    block: DeoxysPendingBlock,
    state_diff: &StateDiff,
    chain_id: ChainId,
    block_number: u64,
) -> Result<BlockImportResult, BlockImportError> {
    let validation_context = ValidationContext {
        trust_transaction_hashes: true, // no need to recompute tx hashes
        chain_id,
        trust_global_tries: false,
    };

    let DeoxysPendingBlock { info, inner } = block;
    let DeoxysPendingBlockInfo { header, tx_hashes: _tx_hashes } = info;

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
                declared_classes: vec![],
                commitments: Default::default(), // the block importer will compute the commitments for us
            },
            validation_context.clone(),
        )
        .await?;

    importer.verify_apply(block, validation_context.clone()).await
}
