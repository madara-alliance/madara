use deoxys_runtime::opaque::DBlockT;
use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilterWithPage, EventsPage};
use starknet_ff::FieldElement;

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::StarknetRpcApiError;
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
pub async fn get_events<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    filter: EventFilterWithPage,
) -> RpcResult<EventsPage>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let from_address = filter.event_filter.address.map(Felt252Wrapper::from);
    let keys = filter.event_filter.keys.unwrap_or_default();
    let chunk_size = filter.result_page_request.chunk_size;

    if keys.len() > MAX_EVENTS_KEYS {
        return Err(StarknetRpcApiError::TooManyKeysInFilter.into());
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE as u64 {
        return Err(StarknetRpcApiError::PageSizeTooBig.into());
    }

    // Get the substrate block numbers for the requested range
    let (from_block, to_block, latest_block) =
        block_range(filter.event_filter.from_block, filter.event_filter.to_block, starknet)?;

    let continuation_token = match filter.result_page_request.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|e| {
            error!("Failed to parse continuation token: {:?}", e);
            StarknetRpcApiError::InvalidContinuationToken
        })?,
        None => ContinuationToken { block_n: from_block, event_n: 0 },
    };

    // Verify that the requested range is valid
    if from_block > to_block {
        return Ok(EventsPage { events: vec![], continuation_token: None });
    }

    let from_block = continuation_token.block_n;
    let mut current_block = from_block;

    let mut filtered_events: Vec<EmittedEvent> = Vec::new();

    // Iterate on block range
    while current_block <= to_block {
        let emitted_events = if current_block <= latest_block {
            starknet.get_block_events(BlockId::Number(current_block))?
        } else {
            starknet.get_block_events(BlockId::Tag(BlockTag::Pending))?
        };

        let block_filtered_events: Vec<EmittedEvent> = filter_events_by_params(emitted_events, from_address, &keys);

        if current_block == from_block && (block_filtered_events.len() as u64) < continuation_token.event_n {
            return Err(StarknetRpcApiError::InvalidContinuationToken.into());
        }

        #[allow(clippy::iter_skip_zero)]
        let block_filtered_reduced_events: Vec<EmittedEvent> = block_filtered_events
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

            return Ok(EventsPage { events: filtered_events, continuation_token: token });
        }

        current_block += 1;
    }

    Ok(EventsPage { events: filtered_events, continuation_token: None })
}

/// Helper function to get filter events using address and keys

/// # Arguments
///
/// * `events` - A vector of all events
/// * `address` - Address to use to filter the events
/// * `keys` - Keys to use to filter the events. An event is filtered if any key is present
/// * `max_results` - Optional, indicated the max events that need to be filtered
///
/// # Returns
///
/// * `(block_events: Vec<EventWrapper>, continuation_token: usize)` - A tuple of the filtered
///   events and the first index which still hasn't been processed block_id and an instance of Block
fn filter_events_by_params<'a, 'b: 'a>(
    events: Vec<EmittedEvent>,
    address: Option<Felt252Wrapper>,
    keys: &'a [Vec<FieldElement>],
) -> Vec<EmittedEvent> {
    let mut filtered_events = vec![];

    // Iterate on block events.
    for event in events {
        let match_from_address = address.map_or(true, |addr| addr.0 == event.from_address);
        // Based on https://github.com/starkware-libs/papyrus
        let match_keys = keys
            .iter()
            .enumerate()
            .all(|(i, keys)| event.keys.len() > i && (keys.is_empty() || keys.contains(&event.keys[i])));

        if match_from_address && match_keys {
            filtered_events.push(event);
        }
    }
    filtered_events
}

fn block_range<A, BE, G, C, P, H>(
    from_block: Option<BlockId>,
    to_block: Option<BlockId>,
    starknet: &Starknet<A, BE, G, C, P, H>,
) -> Result<(u64, u64, u64), StarknetRpcApiError>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let latest = starknet.substrate_block_number_from_starknet_block(BlockId::Tag(BlockTag::Latest)).map_err(|e| {
        error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;
    let from = if from_block == Some(BlockId::Tag(BlockTag::Pending)) {
        latest + 1
    } else {
        starknet.substrate_block_number_from_starknet_block(from_block.unwrap_or(BlockId::Number(0))).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?
    };
    let to = if to_block == Some(BlockId::Tag(BlockTag::Pending)) {
        latest + 1
    } else {
        starknet.substrate_block_number_from_starknet_block(from_block.unwrap_or(BlockId::Number(0))).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?
    };
    Ok((from, to, latest))
}
