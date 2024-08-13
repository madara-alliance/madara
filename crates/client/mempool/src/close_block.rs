use dc_db::DeoxysBackend;
use dc_sync::{
    commitments::update_tries_and_compute_state_root,
    convert::{compute_commitments_for_block, BlockCommitments},
};
use dp_block::{
    header::PendingHeader, DeoxysBlock, DeoxysBlockInfo, DeoxysPendingBlock, DeoxysPendingBlockInfo, Header,
};
use dp_state_update::StateDiff;
use starknet_types_core::felt::Felt;

pub fn close_block(
    backend: &DeoxysBackend,
    block: DeoxysPendingBlock,
    state_diff: &StateDiff,
    chain_id: Felt,
    block_number: u64,
) -> DeoxysBlock {
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

    let (global_state_root, block_commitments) = rayon::join(
        || update_tries_and_compute_state_root(backend, state_diff, block_number),
        || compute_commitments_for_block(&inner, state_diff, protocol_version, chain_id, block_number),
    );

    let BlockCommitments {
        transaction_commitment,
        transaction_count,
        event_commitment,
        event_count,
        receipt_commitment,
        state_diff_commitment,
        state_diff_length,
        tx_hashes,
    } = block_commitments;

    let header = Header {
        parent_block_hash,
        sequencer_address,
        block_timestamp,
        protocol_version,
        l1_gas_price,
        l1_da_mode,

        // Extra fields.
        block_number,

        // Commitments.
        global_state_root,
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
        state_diff_length,
        state_diff_commitment,
        receipt_commitment,
    };

    let block_hash = header.compute_hash(chain_id);

    DeoxysBlock { info: DeoxysBlockInfo { header, block_hash, tx_hashes }, inner }
}
