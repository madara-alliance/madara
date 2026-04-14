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

    fn requires_local_resume(&self) -> bool {
        self.allowed_addresses.is_some() && self.db_from_address.is_none()
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
        &requested_continuation_token,
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
    requested_continuation_token: &ContinuationToken,
    to_block_n: u64,
    keys: Option<Vec<Vec<Felt>>>,
    chunk_size: usize,
    address_filter: AddressScanFilter,
) -> StarknetRpcResult<Vec<EventWithInfo>> {
    let requested_event_n = usize::try_from(requested_continuation_token.event_n)
        .map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?;
    let mut scan_block = requested_continuation_token.block_number;
    let mut scan_event_n = if address_filter.requires_local_resume() { 0 } else { requested_event_n };
    let mut remaining_matches_to_skip_in_requested_block =
        if address_filter.requires_local_resume() { requested_event_n } else { 0 };
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

        for event_info in &batch {
            if !address_filter.matches(&event_info) {
                continue;
            }

            if remaining_matches_to_skip_in_requested_block > 0 {
                if event_info.block_number != requested_continuation_token.block_number {
                    return Err(StarknetRpcApiError::InvalidContinuationToken);
                }

                remaining_matches_to_skip_in_requested_block -= 1;
                continue;
            }

            events_infos.push(event_info.clone());
            if events_infos.len() > chunk_size {
                break;
            }
        }

        (scan_block, scan_event_n) = advance_scan_cursor(&batch, scan_block, scan_event_n)?;
    }

    if remaining_matches_to_skip_in_requested_block > 0 {
        return Err(StarknetRpcApiError::InvalidContinuationToken);
    }

    Ok(events_infos)
}

fn advance_scan_cursor(
    batch: &[EventWithInfo],
    scan_block: u64,
    scan_event_n: usize,
) -> StarknetRpcResult<(u64, usize)> {
    let Some(last_block_number) = batch.last().map(|event_info| event_info.block_number) else {
        return Ok((scan_block, scan_event_n));
    };

    let events_in_last_block =
        batch.iter().rev().take_while(|event_info| event_info.block_number == last_block_number).count();

    let next_event_n = if last_block_number == scan_block {
        scan_event_n.checked_add(events_in_last_block).ok_or(StarknetRpcApiError::InternalServerError)?
    } else {
        events_in_last_block
    };

    Ok((last_block_number, next_event_n))
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn dummy_event(block_number: u64, event_index_in_block: u64) -> EventWithInfo {
        EventWithInfo {
            event: mp_receipt::Event { from_address: Felt::ZERO, keys: vec![], data: vec![] },
            block_number,
            block_hash: None,
            transaction_hash: Felt::ZERO,
            transaction_index: 0,
            event_index_in_block,
            in_preconfirmed: false,
        }
    }

    #[rstest]
    #[case(&[(7, 0), (7, 2), (7, 5)], 2, ContinuationToken { block_number: 7, event_n: 0 }, Some(ContinuationToken { block_number: 7, event_n: 2 }))]
    #[case(&[(7, 0), (8, 0), (8, 1)], 2, ContinuationToken { block_number: 7, event_n: 0 }, Some(ContinuationToken { block_number: 8, event_n: 1 }))]
    #[case(&[(7, 0), (7, 2)], 2, ContinuationToken { block_number: 7, event_n: 0 }, None)]
    fn build_events_chunk_tracks_filtered_offsets_per_block(
        #[case] raw_positions: &[(u64, u64)],
        #[case] page_size: usize,
        #[case] previous_token: ContinuationToken,
        #[case] expected: Option<ContinuationToken>,
    ) {
        let events_infos = raw_positions
            .iter()
            .copied()
            .map(|(block_number, event_index)| dummy_event(block_number, event_index))
            .collect::<Vec<_>>();

        let continuation_token = continuation_token_from_page(&events_infos, page_size, &previous_token);

        assert_eq!(continuation_token, expected);
    }

    #[rstest]
    #[case(&[(4, 0), (4, 0)], 4, 0, (4, 2))]
    #[case(&[(0, 0), (0, 1), (4, 0)], 0, 0, (4, 1))]
    #[case(&[(4, 5), (4, 9)], 4, 2, (4, 4))]
    fn advance_scan_cursor_counts_matching_events_instead_of_reusing_event_index(
        #[case] raw_positions: &[(u64, u64)],
        #[case] scan_block: u64,
        #[case] scan_event_n: usize,
        #[case] expected: (u64, usize),
    ) {
        let batch = raw_positions
            .iter()
            .copied()
            .map(|(block_number, event_index)| dummy_event(block_number, event_index))
            .collect::<Vec<_>>();

        let next_cursor =
            advance_scan_cursor(&batch, scan_block, scan_event_n).expect("cursor advancement should succeed");

        assert_eq!(next_cursor, expected);
    }

    #[rstest]
    fn build_events_chunk_truncates_page_but_preserves_filtered_offset() {
        let chunk = build_events_chunk(
            vec![dummy_event(7, 0), dummy_event(7, 2), dummy_event(7, 5)],
            2,
            &ContinuationToken { block_number: 7, event_n: 0 },
        );

        assert_eq!(chunk.events.len(), 2);
        assert_eq!(chunk.events[0].event_index, 0);
        assert_eq!(chunk.events[1].event_index, 2);
        assert_eq!(chunk.continuation_token.as_deref(), Some("7-2"));
    }
}
