use starknet_types_core::felt::Felt;

use crate::{MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};

/// Represents a Starknet event along with contextual metadata.
///
/// This structure is useful for scenarios where the raw event data needs to be enriched with additional context — particularly when the event is not retrieved directly from a transaction receipt or block.
#[derive(Clone, Debug)]
pub struct EventWithInfo {
    /// The raw event data.
    pub event: mp_receipt::Event,

    /// The number of the block in which the event was emitted, None for pending blocks.
    pub block_number: Option<u64>,

    /// The hash of the block where the event occurred, None for pending blocks.
    pub block_hash: Option<Felt>,

    /// The hash of the transaction that emitted this event.
    pub transaction_hash: Felt,

    /// The index of the event in the block (not in the transaction).
    /// This allows deterministic ordering of events within the block.
    pub event_index_in_block: usize,
}

impl From<EventWithInfo> for mp_rpc::EmittedEvent {
    fn from(event_with_info: EventWithInfo) -> Self {
        mp_rpc::EmittedEvent {
            event: event_with_info.event.into(),
            block_hash: event_with_info.block_hash,
            block_number: event_with_info.block_number,
            transaction_hash: event_with_info.transaction_hash,
        }
    }
}

/// Extracts and iterates over all events emitted within a block.
///
/// This function processes all transactions in a given block (whether pending or confirmed)
/// and returns an iterator over their emitted events. Each event is enriched with its
/// contextual information including block details and the transaction that generated it.
///
/// # Arguments
///
/// * `block` - A reference to either a pending or confirmed block (`MadaraMaybePendingBlock`)
///
/// # Returns
///
/// Returns an iterator yielding `EmittedEvent` items. Each item contains:
/// - The event data (from address, keys, and associated data)
/// - Block context (hash and number, if the block is confirmed)
/// - Transaction hash that generated the event
pub fn drain_block_events(block: MadaraMaybePendingBlock) -> impl Iterator<Item = EventWithInfo> {
    let (block_hash, block_number) = match &block.info {
        MadaraMaybePendingBlockInfo::Pending(_) => (None, None),
        MadaraMaybePendingBlockInfo::NotPending(block) => (Some(block.block_hash), Some(block.header.block_number)),
    };

    let tx_hash_and_events = block.inner.receipts.into_iter().flat_map(|receipt| {
        let tx_hash = receipt.transaction_hash();
        receipt.into_events().into_iter().map(move |events| (tx_hash, events))
    });

    tx_hash_and_events.enumerate().map(move |(event_index, (transaction_hash, event))| EventWithInfo {
        event,
        block_number,
        event_index_in_block: event_index,
        block_hash,
        transaction_hash,
    })
}

/// Filters events based on the provided address and keys.
///
/// This function checks if an event matches the given address and keys.
/// If an address is provided, the event must originate from that address.
/// The event's keys must match the provided keys pattern.
///
/// # Arguments
///
/// * `event` - A reference to the event to be filtered.
/// * `address` - An optional address that the event must originate from.
/// * `keys` - An optional slice of key patterns that the event's keys must match.
///
/// # Returns
///
/// * `true` if the event matches the address and keys pattern.
/// * `false` otherwise.
#[inline]
pub fn event_match_filter(event: &mp_receipt::Event, address: Option<&Felt>, keys: Option<&[Vec<Felt>]>) -> bool {
    // Check if the event's address matches the provided address, if any.
    if let Some(addr) = address {
        if addr != &event.from_address {
            return false;
        }
    }

    // If keys are not provided, return true.
    if let Some(keys) = keys {
        // Check if the number of keys in the event matches the number of provided key patterns.
        if keys.len() > event.keys.len() {
            return false;
        }

        // Check if each key in the event matches the corresponding key pattern.
        // Use iterators to traverse both keys and event.event_content.keys simultaneously.
        for (pattern, key) in keys.iter().zip(event.keys.iter()) {
            if !pattern.is_empty() && !pattern.contains(key) {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_receipt::Event;
    use rstest::*;

    #[fixture]
    fn base_event() -> Event {
        Event {
            from_address: Felt::from_hex_unchecked("0x1234"),
            keys: vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")],
            data: vec![Felt::from_hex_unchecked("0x5678")],
        }
    }

    #[fixture]
    fn matching_address() -> Felt {
        Felt::from_hex_unchecked("0x1234")
    }

    #[fixture]
    fn non_matching_address() -> Felt {
        Felt::from_hex_unchecked("0x5678")
    }

    #[fixture]
    fn matching_keys() -> Vec<Vec<Felt>> {
        vec![vec![Felt::from_hex_unchecked("0x1")], vec![Felt::from_hex_unchecked("0x2")]]
    }

    #[fixture]
    fn matching_keys_empty() -> Vec<Vec<Felt>> {
        vec![vec![], vec![]]
    }

    #[fixture]
    fn non_matching_keys() -> Vec<Vec<Felt>> {
        vec![vec![Felt::from_hex_unchecked("0x1")], vec![Felt::from_hex_unchecked("0x3")]]
    }

    #[rstest]
    fn test_address_and_keys_match(base_event: Event, matching_address: Felt, matching_keys: Vec<Vec<Felt>>) {
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&matching_keys)));
    }

    #[rstest]
    fn test_address_and_empty_keys_match(
        base_event: Event,
        matching_address: Felt,
        matching_keys_empty: Vec<Vec<Felt>>,
    ) {
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&matching_keys_empty)));
    }

    #[rstest]
    fn test_address_does_not_match(base_event: Event, non_matching_address: Felt, matching_keys: Vec<Vec<Felt>>) {
        assert!(!event_match_filter(&base_event, Some(&non_matching_address), Some(&matching_keys)));
    }

    #[rstest]
    fn test_keys_do_not_match(base_event: Event, matching_address: Felt, non_matching_keys: Vec<Vec<Felt>>) {
        assert!(!event_match_filter(&base_event, Some(&matching_address), Some(&non_matching_keys)));
    }

    #[rstest]
    fn test_no_address_provided(base_event: Event, matching_keys: Vec<Vec<Felt>>) {
        assert!(event_match_filter(&base_event, None, Some(&matching_keys)));
    }

    #[rstest]
    fn test_no_keys_provided(base_event: Event, matching_address: Felt) {
        assert!(event_match_filter(&base_event, Some(&matching_address), None));
    }

    #[rstest]
    fn test_keys_with_pattern(base_event: Event, matching_address: Felt) {
        // [0x1 | 0x2, 0x2]
        let keys = vec![
            vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2")],
            vec![Felt::from_hex_unchecked("0x2")],
        ];
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&keys)));

        // [_, 0x3 | 0x2]
        let keys = vec![vec![], vec![Felt::from_hex_unchecked("0x3"), Felt::from_hex_unchecked("0x2")]];
        assert!(event_match_filter(&base_event, Some(&matching_address), Some(&keys)));
    }
}
