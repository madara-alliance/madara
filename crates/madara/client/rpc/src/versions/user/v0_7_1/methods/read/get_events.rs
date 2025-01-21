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
        if keys.len() > MAX_EVENTS_KEYS {
            return Err(StarknetRpcApiError::TooManyKeysInFilter);
        }
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE as u64 {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    // Get the block numbers for the requested range
    let (from_block, to_block, latest_block) = block_range(starknet, filter.from_block, filter.to_block)?;

    let continuation_token = match filter.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?,
        None => ContinuationToken { block_n: from_block, event_n: 0 },
    };

    // Verify that the requested range is valid
    if from_block > to_block {
        return Ok(EventsChunk { events: vec![], continuation_token: None });
    }

    let from_block = continuation_token.block_n;
    let mut filtered_events: Vec<EmittedEvent<Felt>> = Vec::new();

    let iter_filter = starknet
        .backend
        .get_event_filter_stream(from_block)
        .or_internal_server_error("Error getting event filter stream")?;

    let key_filter = EventBloomSearcher::new(from_address.as_ref(), keys.as_deref());

    for filter_block in iter_filter {
        let (current_block, filter) = filter_block.or_internal_server_error("Error getting next filter block")?;

        if current_block > latest_block {
            break;
        }

        if !key_filter.search(&filter) {
            continue;
        }

        let block =
            starknet.get_block(&BlockId::Number(current_block)).or_internal_server_error("Error getting block")?;

        // TODO: take only the events to fill the chunk not all the events
        let block_filtered_events: Vec<EmittedEvent<Felt>> = drain_block_events(block)
            .filter(|event| event_match_filter(&event.event, from_address.as_ref(), keys.as_deref()))
            .collect();

        if current_block == from_block && (block_filtered_events.len() as u64) < continuation_token.event_n {
            return Err(StarknetRpcApiError::InvalidContinuationToken);
        }

        #[allow(clippy::iter_skip_zero)]
        let block_filtered_reduced_events: Vec<EmittedEvent<Felt>> = block_filtered_events
            .into_iter()
            .skip(if current_block == from_block { continuation_token.event_n as usize } else { 0 })
            .take(chunk_size as usize - filtered_events.len())
            .collect();

        let num_events = block_filtered_reduced_events.len();

        filtered_events.extend(block_filtered_reduced_events);

        if filtered_events.len() == chunk_size as usize {
            let event_n =
                if current_block == from_block { continuation_token.event_n + chunk_size } else { num_events as u64 };
            let token = Some(ContinuationToken { block_n: current_block, event_n }.to_string());

            return Ok(EventsChunk { events: filtered_events, continuation_token: token });
        }
        if current_block == to_block {
            break;
        }
    }
    // TODO: handle the case where 'to_block' is pending

    Ok(EventsChunk { events: filtered_events, continuation_token: None })
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
