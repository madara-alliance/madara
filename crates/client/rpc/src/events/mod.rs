use dp_block::DeoxysBlock;
use dp_convert::core_felt::CoreFelt;
use starknet_core::types::EmittedEvent;

use crate::Starknet;

pub fn get_block_events(_starknet: &Starknet, block: &DeoxysBlock, pending: bool) -> Vec<EmittedEvent> {
    let (block_hash, block_number) =
        if pending { (None, None) } else { (Some(block.block_hash().into_core_felt()), Some(block.block_n())) };

    let txs_hashes = block.tx_hashes().iter().map(CoreFelt::into_core_felt).collect::<Vec<_>>();
    let tx_hash_and_events = block.events().iter().flat_map(|ordered_event| {
        let tx_hash = txs_hashes[ordered_event.index() as usize];
        ordered_event.events().iter().map(move |events| (tx_hash, events.clone()))
    });

    tx_hash_and_events
        .map(|(transaction_hash, event)| EmittedEvent {
            from_address: event.from_address.into_core_felt(),
            keys: event.content.keys.into_iter().map(CoreFelt::into_core_felt).collect(),
            data: event.content.data.0.into_iter().map(CoreFelt::into_core_felt).collect(),
            block_hash,
            block_number,
            transaction_hash,
        })
        .collect()
}
