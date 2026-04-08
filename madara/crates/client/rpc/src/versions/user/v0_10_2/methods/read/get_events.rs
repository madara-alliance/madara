use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::{continuation_token_from_page, ContinuationToken};
use crate::Starknet;
use anyhow::Context;
use mc_db::{EventFilter, MadaraStateView};
use mp_block::EventWithInfo;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{AddressFilter, EventFilterWithPageRequest, EventsChunk};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
struct AddressScanFilter {
    db_from_address: Option<Felt>,
    allowed_addresses: Option<HashSet<Felt>>,
}

impl AddressScanFilter {
    fn new(address_filter: Option<&AddressFilter>) -> Self {
        let allowed_addresses = address_filter.and_then(AddressFilter::to_set);
        let db_from_address = allowed_addresses.as_ref().and_then(|addresses| {
            if addresses.len() == 1 {
                addresses.iter().next().copied()
            } else {
                None
            }
        });

        Self { db_from_address, allowed_addresses }
    }

    fn db_from_address(&self) -> Option<Felt> {
        self.db_from_address
    }

    fn matches(&self, event_info: &EventWithInfo) -> bool {
        self.allowed_addresses
            .as_ref()
            .map(|addresses| addresses.contains(&event_info.event.from_address))
            .unwrap_or(true)
    }
}

/// Returns events matching the filter with pagination support.
///
/// v0.10.2: Address filter supports either a single address or an array of addresses.
/// - Single address: Filter events from exactly that address
/// - Array of addresses: Filter events from any of the addresses in the array
/// - Empty array: Match all addresses (no filter)
/// - None: Match all addresses (no filter)
pub fn get_events(starknet: &Starknet, filter: EventFilterWithPageRequest) -> StarknetRpcResult<EventsChunk> {
    let view = starknet.backend.view_on_latest();
    let EventFilterWithPageRequest { address, from_block, keys, to_block, chunk_size, continuation_token } = filter;
    let chunk_size = validate_filter_limits(chunk_size as usize, keys.as_deref())?;
    let (from_block_n, to_block_n) = resolve_block_range(starknet, &view, from_block, to_block)?;

    if from_block_n > to_block_n {
        return Ok(empty_events_chunk());
    }

    let requested_continuation_token = parse_continuation_token(continuation_token, from_block_n)?;
    let events_infos = collect_matching_events(
        &view,
        requested_continuation_token.block_number,
        requested_continuation_token.event_n as usize,
        to_block_n,
        keys,
        chunk_size,
        AddressScanFilter::new(address.as_ref()),
    )?;

    Ok(build_events_chunk(events_infos, chunk_size, &requested_continuation_token))
}

fn validate_filter_limits(chunk_size: usize, keys: Option<&[Vec<Felt>]>) -> StarknetRpcResult<usize> {
    if keys.map(|patterns| patterns.iter().map(|pattern| pattern.len()).sum::<usize>()).unwrap_or(0) > MAX_EVENTS_KEYS {
        return Err(StarknetRpcApiError::TooManyKeysInFilter);
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    Ok(chunk_size)
}

fn resolve_block_range(
    starknet: &Starknet,
    view: &MadaraStateView,
    from_block: Option<BlockId>,
    to_block: Option<BlockId>,
) -> StarknetRpcResult<(u64, u64)> {
    let from_block_n = from_block.map(|block_id| resolve_block_n(starknet, block_id)).transpose()?.unwrap_or(0);
    let to_block_n = to_block
        .map(|block_id| resolve_block_n(starknet, block_id))
        .transpose()?
        .unwrap_or_else(|| view.latest_block_n().unwrap_or(0));

    Ok((from_block_n, to_block_n))
}

fn resolve_block_n(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<u64> {
    Ok(starknet.resolve_view_on(block_id)?.latest_block_n().unwrap_or(0))
}

fn parse_continuation_token(
    continuation_token: Option<String>,
    from_block_n: u64,
) -> StarknetRpcResult<ContinuationToken> {
    match continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken),
        None => Ok(ContinuationToken { block_number: from_block_n, event_n: 0 }),
    }
}

fn collect_matching_events(
    view: &MadaraStateView,
    from_block_n: u64,
    from_event_n: usize,
    to_block_n: u64,
    keys: Option<Vec<Vec<Felt>>>,
    chunk_size: usize,
    address_filter: AddressScanFilter,
) -> StarknetRpcResult<Vec<EventWithInfo>> {
    let mut scan_block = from_block_n;
    let mut scan_event_n = from_event_n;
    let mut events_infos = Vec::with_capacity(chunk_size + 1);

    while events_infos.len() <= chunk_size {
        let batch = view
            .get_events(EventFilter {
                start_block: scan_block,
                start_event_index: scan_event_n,
                end_block: to_block_n,
                from_address: address_filter.db_from_address(),
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
            if !address_filter.matches(&event_info) {
                continue;
            }

            events_infos.push(event_info);
            if events_infos.len() > chunk_size {
                break;
            }
        }
    }

    Ok(events_infos)
}

fn build_events_chunk(
    mut events_infos: Vec<EventWithInfo>,
    chunk_size: usize,
    requested_continuation_token: &ContinuationToken,
) -> EventsChunk {
    let continuation_token = continuation_token_from_page(&events_infos, chunk_size, requested_continuation_token);
    if continuation_token.is_some() {
        events_infos.truncate(chunk_size);
    }

    EventsChunk {
        events: events_infos.into_iter().map(Into::into).collect(),
        continuation_token: continuation_token.map(|token| token.to_string()),
    }
}

fn empty_events_chunk() -> EventsChunk {
    EventsChunk { events: vec![], continuation_token: None }
}
