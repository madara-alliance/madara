use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::ContinuationToken;
use crate::Starknet;
use anyhow::Context;
use mc_db::EventFilter;
use mp_block::{BlockId, BlockTag, EventWithInfo};
use mp_rpc::{EmittedEvent, Event, EventContent, EventFilterWithPageRequest, EventsChunk};

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
pub fn get_events(starknet: &Starknet, filter: EventFilterWithPageRequest) -> StarknetRpcResult<EventsChunk> {
    let from_address = filter.address;
    let keys = filter.keys;
    let chunk_size = filter.chunk_size as usize;

    if keys.as_ref().map(|k| k.iter().map(|pattern| pattern.len()).sum()).unwrap_or(0) > MAX_EVENTS_KEYS {
        return Err(StarknetRpcApiError::TooManyKeysInFilter);
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE {
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
    let from_event_n = continuation_token.event_n as usize;

    let mut events_infos = starknet
        .backend
        .view_on_latest()
        .get_events(EventFilter {
            start_block: from_block,
            start_event_index: from_event_n,
            end_block: to_block,
            from_address,
            keys_pattern: keys,
            max_events: chunk_size + 1,
        })
        .context("Error getting filtered events")?;

    let mut continuation_token = None;
    if events_infos.len() > chunk_size {
        continuation_token = events_infos.pop().and_then(|event_info| match event_info {
            EventWithInfo { block_number: Some(block_n), event_index_in_block, .. } => {
                Some(ContinuationToken { block_n, event_n: (event_index_in_block + 1) as u64 })
            }
            _ => None,
        });
    }

    Ok(EventsChunk {
        events: events_infos
            .into_iter()
            .map(|event_info| EmittedEvent {
                event: Event {
                    from_address: event_info.event.from_address,
                    event_content: EventContent { keys: event_info.event.keys, data: event_info.event.data },
                },
                block_hash: event_info.block_hash,
                block_number: event_info.block_number,
                transaction_hash: event_info.transaction_hash,
            })
            .collect(),
        continuation_token: continuation_token.map(|token| token.to_string()),
    })
}

fn block_range(
    starknet: &Starknet,
    from_block: Option<BlockId>,
    to_block: Option<BlockId>,
) -> StarknetRpcResult<(u64, u64, u64)> {
    let latest_block_n = starknet.backend.latest_confirmed_block_n().ok_or(StarknetRpcApiError::NoBlocks)?;
    let from_block_n = match from_block {
        Some(BlockId::Tag(BlockTag::Pending)) => latest_block_n + 1,
        Some(block_id) => starknet.backend.block_view(&block_id)?.block_number(),
        None => 0,
    };
    let to_block_n = match to_block {
        Some(BlockId::Tag(BlockTag::Pending)) => latest_block_n + 1,
        Some(block_id) => starknet.backend.block_view(&block_id)?.block_number(),
        None => latest_block_n,
    };
    Ok((from_block_n, to_block_n, latest_block_n))
}
