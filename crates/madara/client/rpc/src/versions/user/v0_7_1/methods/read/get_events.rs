use mp_block::{BlockId, BlockTag, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
use mp_bloom_filter::EventBloomSearcher;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{EmittedEvent, Event, EventContent, EventFilterWithPageRequest, EventsChunk};

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::ContinuationToken;
use crate::utils::event_match_filter;
use crate::utils::ResultExt;
use crate::Starknet;

/// Returns all events matching the given filter.
///
/// This function retrieves all event objects that match the conditions specified in the
/// provided event filter. The filter can include various criteria such as contract addresses,
/// event types, and block ranges. The function supports pagination through the result page
/// request schema.
///
/// ### Arguments
///
/// * `filter` - The conditions used to filter the returned events. The filter is a combination of
///   an event filter and a result page request, allowing for precise control over which events are
///   returned and in what quantity.
///
/// ### Returns
///
/// Returns a chunk of event objects that match the filter criteria, encapsulated in an
/// `EventsChunk` type. The chunk includes details about the events, such as their data, the
/// block in which they occurred, and the transaction that triggered them. In case of
/// errors, such as `PAGE_SIZE_TOO_BIG`, `INVALID_CONTINUATION_TOKEN`, `BLOCK_NOT_FOUND`, or
/// `TOO_MANY_KEYS_IN_FILTER`, returns a `StarknetRpcApiError` indicating the specific issue.
pub async fn get_events(
    starknet: &Starknet,
    filter: EventFilterWithPageRequest<Felt>,
) -> StarknetRpcResult<EventsChunk<Felt>> {
    // TODO: use a continuation token like BlockN_EventN were EventN is the index of the event in the block and not the index of filtered events

    let from_address = filter.address;
    let keys = filter.keys;
    let chunk_size = filter.chunk_size;

    if let Some(keys) = &keys {
        if keys.iter().flatten().count() > MAX_EVENTS_KEYS {
            return Err(StarknetRpcApiError::TooManyKeysInFilter);
        }
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE as u64 {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    // Get the block numbers for the requested range
    let (from_block, to_block, _) = block_range(starknet, filter.from_block, filter.to_block)?;

    let continuation_token = match filter.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?,
        None => ContinuationToken { block_n: from_block, event_n: 0 },
    };

    // Verify that the requested range is valid
    if from_block > to_block {
        return Ok(EventsChunk { events: vec![], continuation_token: None });
    }

    let from_block = continuation_token.block_n;
    let mut events_chunk: Vec<EmittedEvent<Felt>> = Vec::with_capacity(chunk_size as usize);

    let key_filter = EventBloomSearcher::new(from_address.as_ref(), keys.as_deref());

    let filter_event_stream = starknet
        .backend
        .get_event_filter_stream(from_block)
        .or_internal_server_error("Error getting event filter stream")?;

    for filter_block in filter_event_stream {
        // Attempt to retrieve the next block and its bloom filter.
        // Only blocks with events have a bloom filter.
        let (current_block, bloom_filter) = filter_block.or_internal_server_error("Error getting next filter block")?;

        // Stop processing if the current block exceeds the requested range.
        // - `latest_block`: Ensures we do not process beyond the latest finalized block.
        // - `to_block`: Ensures we do not go beyond the user-specified range.
        if current_block > to_block {
            break;
        }

        // Use the bloom filter to quickly check if the block might contain relevant events.
        // - This avoids unnecessary block retrieval if no matching events exist.
        if !key_filter.search(&bloom_filter) {
            continue;
        }

        // Retrieve the full block data since we now suspect it contains relevant events.
        let block =
            starknet.get_block(&BlockId::Number(current_block)).or_internal_server_error("Error getting block")?;

        let mut last_event_index = 0;

        for (idx, event) in drain_block_events(block)
            .enumerate()
            // Skip events that have already been processed if we are resuming from a continuation token.
            // Otherwise, start from the beginning of the block.
            .skip(if current_block == from_block { continuation_token.event_n as usize } else { 0 })

            // Filter events based on the given event filter criteria (address, keys).
            .filter(|(_, event)| event_match_filter(&event.event, from_address.as_ref(), keys.as_deref()))

            // Take exactly enough events to fill the requested chunk size, plus one extra event.
            // The extra event is used to determine if the block has more matching events.
            // - If an extra event is found, it means there are still unprocessed events in this block.
            //   -> The continuation token should point to this block and the next event index.
            // - If no extra event is found, it means all matching events in this block have been retrieved.
            //   -> The continuation token should move to the next block.
            .take(chunk_size as usize - events_chunk.len() + 1)
        {
            if events_chunk.len() < chunk_size as usize {
                events_chunk.push(event);
                last_event_index = idx;
            } else {
                // If a new event was found, it means there are more events that match the filter.
                return Ok(EventsChunk {
                    events: events_chunk,
                    continuation_token: Some(
                        ContinuationToken { block_n: current_block, event_n: (last_event_index + 1) as u64 }
                            .to_string(),
                    ),
                });
            }
        }
    }

    Ok(EventsChunk { events: events_chunk, continuation_token: None })
}

fn block_range(
    starknet: &Starknet,
    from_block: Option<BlockId>,
    to_block: Option<BlockId>,
) -> StarknetRpcResult<(u64, u64, u64)> {
    let latest_block_n = starknet.get_block_n(&BlockId::Tag(BlockTag::Latest))?;
    let from_block_n = match from_block {
        Some(BlockId::Tag(BlockTag::Pending)) => latest_block_n + 1,
        Some(block_id) => starknet.get_block_n(&block_id)?,
        None => 0,
    };
    let to_block_n = match to_block {
        Some(BlockId::Tag(BlockTag::Pending)) => latest_block_n + 1,
        Some(block_id) => starknet.get_block_n(&block_id)?,
        None => latest_block_n,
    };
    Ok((from_block_n, to_block_n, latest_block_n))
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
/// Returns an iterator yielding `EmittedEvent<Felt>` items. Each item contains:
/// - The event data (from address, keys, and associated data)
/// - Block context (hash and number, if the block is confirmed)
/// - Transaction hash that generated the event
pub fn drain_block_events(block: MadaraMaybePendingBlock) -> impl Iterator<Item = EmittedEvent<Felt>> {
    let (block_hash, block_number) = match &block.info {
        MadaraMaybePendingBlockInfo::Pending(_) => (None, None),
        MadaraMaybePendingBlockInfo::NotPending(block) => (Some(block.block_hash), Some(block.header.block_number)),
    };

    let tx_hash_and_events = block.inner.receipts.into_iter().flat_map(|receipt| {
        let tx_hash = receipt.transaction_hash();
        receipt.into_events().into_iter().map(move |events| (tx_hash, events))
    });

    tx_hash_and_events.map(move |(transaction_hash, event)| EmittedEvent {
        event: Event {
            from_address: event.from_address,
            event_content: EventContent { keys: event.keys, data: event.data },
        },
        block_hash,
        block_number,
        transaction_hash,
    })
}
