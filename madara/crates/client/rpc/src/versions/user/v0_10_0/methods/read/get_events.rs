use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::ContinuationToken;
use crate::Starknet;
use anyhow::Context;
use mc_db::EventFilter;
use mp_block::EventWithInfo;
use mp_rpc::v0_10_0::{EventFilterWithPageRequest, EventsChunk};

/// Returns events matching the filter with pagination support.
///
/// v0.10.0: Events include `transaction_index` and `event_index`.
pub fn get_events(starknet: &Starknet, filter: EventFilterWithPageRequest) -> StarknetRpcResult<EventsChunk> {
    let from_address = filter.address;
    let keys = filter.keys;
    let chunk_size = filter.chunk_size as usize;

    let view = starknet.backend.view_on_latest();

    if keys.as_ref().map(|k| k.iter().map(|pattern| pattern.len()).sum()).unwrap_or(0) > MAX_EVENTS_KEYS {
        return Err(StarknetRpcApiError::TooManyKeysInFilter);
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    let from_block_n = match filter.from_block {
        Some(block_id) => starknet.resolve_view_on(block_id)?.latest_block_n().unwrap_or(0),
        None => 0,
    };
    let to_block_n = match filter.to_block {
        Some(block_id) => starknet.resolve_view_on(block_id)?.latest_block_n().unwrap_or(0),
        None => view.latest_block_n().unwrap_or(0),
    };

    let continuation_token = match filter.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?,
        None => ContinuationToken { block_number: from_block_n, event_n: 0 },
    };

    if from_block_n > to_block_n {
        return Ok(EventsChunk { events: vec![], continuation_token: None });
    }

    let from_block = continuation_token.block_number;
    let from_event_n = continuation_token.event_n as usize;

    let mut events_infos = view
        .get_events(EventFilter {
            start_block: from_block,
            start_event_index: from_event_n,
            end_block: to_block_n,
            from_address,
            keys_pattern: keys,
            max_events: chunk_size + 1,
        })
        .context("Error getting filtered events")?;

    let mut continuation_token = None;
    if events_infos.len() > chunk_size {
        continuation_token = events_infos.pop().map(|EventWithInfo { block_number, event_index_in_block, .. }| {
            ContinuationToken { block_number, event_n: (event_index_in_block + 1) }
        });
    }

    Ok(EventsChunk {
        events: events_infos.into_iter().map(|event_info| event_info.into()).collect(),
        continuation_token: continuation_token.map(|token| token.to_string()),
    })
}
