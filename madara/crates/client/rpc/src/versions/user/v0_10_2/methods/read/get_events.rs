use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::ContinuationToken;
use crate::Starknet;
use anyhow::Context;
use mc_db::EventFilter;
use mp_block::EventWithInfo;
use mp_rpc::v0_10_2::{EventFilterWithPageRequest, EventsChunk};

/// Returns events matching the filter with pagination support.
///
/// v0.10.2: Address filter supports either a single address or an array of addresses.
/// - Single address: Filter events from exactly that address
/// - Array of addresses: Filter events from any of the addresses in the array
/// - Empty array: Match all addresses (no filter)
/// - None: Match all addresses (no filter)
pub fn get_events(starknet: &Starknet, filter: EventFilterWithPageRequest) -> StarknetRpcResult<EventsChunk> {
    let chunk_size = filter.chunk_size as usize;
    let address_filter = filter.address;
    let keys = filter.keys;

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

    let db_from_address = address_filter.as_ref().and_then(|filter| match filter {
        mp_rpc::v0_10_2::AddressFilter::Single(address) => Some(*address),
        mp_rpc::v0_10_2::AddressFilter::Multiple(_) => None,
    });

    let mut scan_block = from_block;
    let mut scan_event_n = from_event_n;
    let mut events_infos = Vec::with_capacity(chunk_size + 1);
    let mut overflow = None;

    while events_infos.len() <= chunk_size {
        let batch = view
            .get_events(EventFilter {
                start_block: scan_block,
                start_event_index: scan_event_n,
                end_block: to_block_n,
                from_address: db_from_address,
                keys_pattern: keys.clone(),
                max_events: chunk_size + 1,
            })
            .context("Error getting filtered events")?;

        if batch.is_empty() {
            break;
        }

        if let Some(last) = batch.last() {
            scan_block = last.block_number;
            scan_event_n =
                usize::try_from(last.event_index_in_block + 1).map_err(|_| StarknetRpcApiError::InternalServerError)?;
        }

        for event_info in batch {
            let matches_address =
                address_filter.as_ref().map(|filter| filter.matches(&event_info.event.from_address)).unwrap_or(true);

            if !matches_address {
                continue;
            }

            if events_infos.len() == chunk_size {
                overflow = Some(event_info);
                break;
            }

            events_infos.push(event_info);
        }

        if overflow.is_some() {
            break;
        }
    }

    let mut continuation_token = None;
    if let Some(EventWithInfo { block_number, event_index_in_block, .. }) = overflow {
        continuation_token = Some(ContinuationToken { block_number, event_n: (event_index_in_block + 1) });
    } else if events_infos.len() > chunk_size {
        continuation_token = events_infos.pop().map(|EventWithInfo { block_number, event_index_in_block, .. }| {
            ContinuationToken { block_number, event_n: (event_index_in_block + 1) }
        });
    }

    Ok(EventsChunk {
        events: events_infos.into_iter().map(|event_info| event_info.into()).collect(),
        continuation_token: continuation_token.map(|token| token.to_string()),
    })
}
