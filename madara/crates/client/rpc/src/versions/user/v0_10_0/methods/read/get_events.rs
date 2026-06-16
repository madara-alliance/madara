use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::{continuation_token_from_page, ContinuationToken};
use crate::Starknet;
use anyhow::Context;
use mc_db::EventFilter;
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
    if chunk_size == 0 || chunk_size > MAX_EVENTS_CHUNK_SIZE {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    let from_block_n = match filter.from_block {
        Some(block_id) => starknet.resolve_event_from_block_bound(block_id)?,
        None => 0,
    };
    let to_block_n = match filter.to_block {
        Some(block_id) => starknet.resolve_event_to_block_bound(block_id)?,
        None => view.latest_block_n().unwrap_or(0),
    };

    let requested_continuation_token = match filter.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?,
        None => ContinuationToken { block_number: from_block_n, event_n: 0 },
    };

    if from_block_n > to_block_n {
        return Ok(EventsChunk { events: vec![], continuation_token: None });
    }

    let from_block = requested_continuation_token.block_number;
    let from_event_n = requested_continuation_token.event_n as usize;

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

    let continuation_token = continuation_token_from_page(&events_infos, chunk_size, &requested_continuation_token);
    if continuation_token.is_some() {
        events_infos.truncate(chunk_size);
    }

    Ok(EventsChunk {
        events: events_infos.into_iter().map(|event_info| event_info.into()).collect(),
        continuation_token: continuation_token.map(|token| token.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::rpc_test_setup;
    use mp_rpc::v0_10_0::BlockId;
    use mp_rpc::v0_9_0::BlockTag;
    use rstest::rstest;

    /// Numeric range bounds past the chain tip return an empty page instead of BLOCK_NOT_FOUND,
    /// like pathfinder and juno: clients paginating by block number at the tip must not error.
    #[rstest]
    fn get_events_beyond_chain_tip_returns_empty_page(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (_backend, rpc) = rpc_test_setup;

        let chunk = get_events(
            &rpc,
            EventFilterWithPageRequest {
                address: None,
                from_block: Some(BlockId::Number(100)),
                to_block: Some(BlockId::Number(200)),
                keys: None,
                chunk_size: 10,
                continuation_token: None,
            },
        )
        .unwrap();

        assert!(chunk.events.is_empty());
        assert!(chunk.continuation_token.is_none());
    }

    #[rstest]
    fn get_events_from_latest_on_empty_chain_returns_block_not_found(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (_backend, rpc) = rpc_test_setup;

        let err = get_events(
            &rpc,
            EventFilterWithPageRequest {
                address: None,
                from_block: Some(BlockId::Tag(BlockTag::Latest)),
                to_block: None,
                keys: None,
                chunk_size: 10,
                continuation_token: None,
            },
        )
        .unwrap_err();

        assert_eq!(err, StarknetRpcApiError::BlockNotFound);
    }
}
