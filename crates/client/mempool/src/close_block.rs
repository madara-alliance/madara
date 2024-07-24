use blockifier::state::cached_state::CommitmentStateDiff;
use dc_db::DeoxysBackend;
use dc_sync::commitments::{
    calculate_tx_and_event_commitments, update_tries_and_compute_state_root, TxAndEventCommitments,
};
use dp_block::{header::PendingHeader, DeoxysBlock, DeoxysBlockInfo, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, DeoxysPendingBlock, DeoxysPendingBlockInfo, Header};
use dp_receipt::{Event, TransactionReceipt};
use starknet_core::types::Felt;

pub fn close_block(
    backend: &DeoxysBackend,
    block: DeoxysPendingBlock,
    csd: CommitmentStateDiff,
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

    let global_state_root = update_tries_and_compute_state_root(backend, csd, block_number);

    fn events(receipts: &[TransactionReceipt]) -> Vec<Event> {
        receipts.iter().flat_map(TransactionReceipt::events).cloned().collect()
    }
    let events = events(&inner.receipts);
    let TxAndEventCommitments { tx_hashes, transaction_commitment: tx_commitment, event_commitment } =
        calculate_tx_and_event_commitments(&inner.transactions, &events, chain_id, block_number);

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
        transaction_count: inner.transactions.len() as _,
        transaction_commitment: tx_commitment,
        event_count: events.len() as _,
        event_commitment,
    };

    let block_hash = header.hash(chain_id);

    let block = DeoxysBlock { info: DeoxysBlockInfo { header, block_hash, tx_hashes }, inner };
    block
}
