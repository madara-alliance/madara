use dp_block::DeoxysBlock;
use dp_convert::ToFelt;
use starknet_core::types::EmittedEvent;

use crate::Starknet;

pub fn get_block_events(_starknet: &Starknet, block: &DeoxysBlock, pending: bool) -> Vec<EmittedEvent> {
    let (block_hash, block_number) =
        if pending { (None, None) } else { (Some(block.block_hash().to_felt()), Some(block.block_n())) };

    let tx_hash_and_events = block.receipts().iter().flat_map(|receipt| {
        let tx_hash = receipt.transaction_hash();
        receipt.events().iter().map(move |events| (tx_hash, events))
    });

    tx_hash_and_events
        .map(|(transaction_hash, event)| EmittedEvent {
            from_address: event.from_address,
            keys: event.keys.clone(),
            data: event.data.clone(),
            block_hash,
            block_number,
            transaction_hash,
        })
        .collect()
}
