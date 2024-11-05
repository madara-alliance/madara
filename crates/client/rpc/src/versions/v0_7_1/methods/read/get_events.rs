use mp_block::{MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilterWithPage, EventsPage, Felt};

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::ContinuationToken;
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
pub async fn get_events(starknet: &Starknet, filter: EventFilterWithPage) -> StarknetRpcResult<EventsPage> {
    let from_address = filter.event_filter.address;
    let keys = filter.event_filter.keys.unwrap_or_default();
    let chunk_size = filter.result_page_request.chunk_size;

    if keys.len() > MAX_EVENTS_KEYS {
        return Err(StarknetRpcApiError::TooManyKeysInFilter);
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE as u64 {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    // TODO: this function doesn't do much, should be hoisted out
    // Get the block numbers for the requested range
    let (from_block, to_block, latest_block) =
        block_range(starknet, filter.event_filter.from_block, filter.event_filter.to_block)?;

    let continuation_token = match filter.result_page_request.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?,
        None => ContinuationToken { block_n: from_block, event_n: 0 },
    };

    // PERF: this is minor but this could happen earlier
    // Verify that the requested range is valid
    if from_block > to_block {
        return Ok(EventsPage { events: vec![], continuation_token: None });
    }

    let from_block = continuation_token.block_n;
    // PERF: this should at least be pre-allocated to some sensible default
    let mut filtered_events: Vec<EmittedEvent> = Vec::new();

    // PERF: we should truncate from_block to the creation block of the contract
    // if it is less than that
    for current_block in from_block..=to_block {
        // PERF: this check can probably be hoisted out of this loop
        let (_pending, block) = if current_block <= latest_block {
            // PERF: This is probably the main bottleneck: we should be able to
            // mitigate this by implementing a db iterator
            (false, starknet.get_block(&BlockId::Number(current_block))?)
        } else {
            (true, starknet.get_block(&BlockId::Tag(BlockTag::Pending))?)
        };

        // PERF: collection needs to be more efficient
        let block_filtered_events: Vec<EmittedEvent> = get_block_events(starknet, &block)
            .into_iter()
            .filter(|event| event_match_filter(event, from_address, &keys))
            .collect();

        // PERF: this condition needs to be moved out the loop as it needs to happen only once
        if current_block == from_block && (block_filtered_events.len() as u64) < continuation_token.event_n {
            return Err(StarknetRpcApiError::InvalidContinuationToken);
        }

        // PERF: same here, hoist this out of the loop
        #[allow(clippy::iter_skip_zero)]
        let block_filtered_reduced_events: Vec<EmittedEvent> = block_filtered_events
            .into_iter()
            .skip(if current_block == from_block { continuation_token.event_n as usize } else { 0 })
            .take(chunk_size as usize - filtered_events.len())
            .collect();

        let num_events = block_filtered_reduced_events.len();

        // PERF: any better way to do this? Pre-allocation should reduce some
        // of the allocations already
        filtered_events.extend(block_filtered_reduced_events);

        if filtered_events.len() == chunk_size as usize {
            let event_n =
                if current_block == from_block { continuation_token.event_n + chunk_size } else { num_events as u64 };
            let token = Some(ContinuationToken { block_n: current_block, event_n }.to_string());

            return Ok(EventsPage { events: filtered_events, continuation_token: token });
        }
    }
    Ok(EventsPage { events: filtered_events, continuation_token: None })
}

#[inline]
fn event_match_filter(event: &EmittedEvent, address: Option<Felt>, keys: &[Vec<Felt>]) -> bool {
    let match_from_address = address.map_or(true, |addr| addr == event.from_address);
    // PERF: HOLY FUCK WE CHECK ALL EVENTS EVEN IF THEY COME FROM THE WRONG
    // ADDRESS
    let match_keys = keys
        .iter()
        .enumerate()
        .all(|(i, keys)| event.keys.len() > i && (keys.is_empty() || keys.contains(&event.keys[i])));
    // PERF: this can be short-circuited
    match_from_address && match_keys
}

// TODO: remove this function, this code can be dealt with manually
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

fn get_block_events(_starknet: &Starknet, block: &MadaraMaybePendingBlock) -> Vec<EmittedEvent> {
    // PERF:: this check can probably be removed by handling pending blocks
    // separatly
    let (block_hash, block_number) = match &block.info {
        MadaraMaybePendingBlockInfo::Pending(_) => (None, None),
        MadaraMaybePendingBlockInfo::NotPending(block) => (Some(block.block_hash), Some(block.header.block_number)),
    };

    let tx_hash_and_events = block.inner.receipts.iter().flat_map(|receipt| {
        let tx_hash = receipt.transaction_hash();
        receipt.events().iter().map(move |events| (tx_hash, events))
    });

    // PERF: clone here is brutal, there must be a way to take ownership of this
    // data
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
