use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BlockTag, EventFilterWithPage, EventsPage};

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::StarknetRpcApiError;
use crate::types::{ContinuationToken, RpcEventFilter};
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
    let latest_block =
        starknet.substrate_block_number_from_starknet_block(BlockId::Tag(BlockTag::Latest)).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
    let from_block = starknet
        .substrate_block_number_from_starknet_block(filter.event_filter.from_block.unwrap_or(BlockId::Number(0)))
        .map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
    let to_block = starknet
        .substrate_block_number_from_starknet_block(
            filter.event_filter.to_block.unwrap_or(BlockId::Tag(BlockTag::Latest)),
        )
        .map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

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

    let to_block = if latest_block > to_block { to_block } else { latest_block };
    let filter = RpcEventFilter { from_block, to_block, from_address, keys, chunk_size, continuation_token };

    starknet.filter_events(filter)
}
