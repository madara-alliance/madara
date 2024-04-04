use jsonrpsee::core::RpcResult;
use log::error;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, EmittedEvent, EventsPage};
use starknet_ff::FieldElement;

use crate::errors::StarknetRpcApiError;
use crate::types::{ContinuationToken, RpcEventFilter};
use crate::utils::get_block_by_block_hash;
use crate::Starknet;

impl<A: ChainApi, BE, G, C, P, H> Starknet<A, BE, G, C, P, H>
where
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    BE: Backend<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    /// Helper function to get Starknet block details
    ///
    /// # Arguments
    ///
    /// * `block_id` - The Starknet block id
    ///
    /// # Returns
    ///
    /// * `(transaction_receipts: Vec<TransactionReceiptWrapper>, block: Block)` - A tuple of the
    ///   block transaction receipts with events in block_id and an instance of Block
    pub fn get_block_events(&self, block_number: u64) -> Result<Vec<EmittedEvent>, StarknetRpcApiError> {
        let substrate_block_hash =
            self.substrate_block_hash_from_starknet_block(BlockId::Number(block_number)).map_err(|e| {
                error!("'{e}'");
                StarknetRpcApiError::BlockNotFound
            })?;

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let block_hash = starknet_block.header().hash::<H>();

        let chain_id = self.client.runtime_api().chain_id(substrate_block_hash).map_err(|_| {
            error!("Failed to retrieve chain id");
            StarknetRpcApiError::InternalServerError
        })?;

        // get txs hashes from cache or compute them
        let block_txs_hashes: Vec<_> = if let Some(tx_hashes) = self.get_cached_transaction_hashes(block_hash.into()) {
            tx_hashes
                .into_iter()
                .map(|h| {
                    Felt252Wrapper::try_from(h)
                        .map(|f| f.0)
                        .map_err(|e| {
                            error!("'{e}'");
                            StarknetRpcApiError::InternalServerError
                        })
                        .unwrap()
                })
                .collect()
        } else {
            starknet_block
                .transactions_hashes::<H>(chain_id, Some(starknet_block.header().block_number))
                .map(FieldElement::from)
                .collect()
        };

        // get txs hashes and events from block
        // the txs hashes are found by the index of the ordered event
        let tx_hash_and_events: Vec<(Felt252Wrapper, _)> = starknet_block
            .events()
            .iter()
            .flat_map(|ordered_event| {
                let tx_hash = block_txs_hashes[ordered_event.index() as usize];
                ordered_event.events().iter().map(move |events| (tx_hash.into(), events.clone()))
            })
            .collect();

        let emitted_events = tx_hash_and_events
            .into_iter()
            .map(|(tx_hash, event)| EmittedEvent {
                from_address: Felt252Wrapper::from(event.from_address).0,
                keys: event.content.keys.into_iter().map(|felt| Felt252Wrapper::from(felt).0).collect(),
                data: event.content.data.0.into_iter().map(|felt| Felt252Wrapper::from(felt).0).collect(),
                block_hash: block_hash.0,
                block_number,
                transaction_hash: tx_hash.0,
            })
            .collect();

        Ok(emitted_events)
    }

    /// Helper function to filter Starknet events provided a RPC event filter
    ///
    /// # Arguments
    ///
    /// * `filter` - The RPC event filter
    ///
    /// # Returns
    ///
    /// * `EventsPage` - The filtered events with continuation token
    pub fn filter_events(&self, filter: RpcEventFilter) -> RpcResult<EventsPage> {
        // get filter values
        let continuation_token = filter.continuation_token;
        // skip blocks with continuation token block number
        let from_block = continuation_token.block_n;
        let mut current_block = from_block;
        let to_block = filter.to_block;
        let from_address = filter.from_address;
        let keys = filter.keys;
        let chunk_size = filter.chunk_size;

        let mut filtered_events: Vec<EmittedEvent> = Vec::new();

        // Iterate on block range
        while current_block <= to_block {
            let emitted_events = self.get_block_events(current_block)?;

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
                let event_n = if current_block == from_block {
                    continuation_token.event_n + chunk_size
                } else {
                    num_events as u64
                };
                let token = Some(ContinuationToken { block_n: current_block, event_n }.to_string());

                return Ok(EventsPage { events: filtered_events, continuation_token: token });
            }

            current_block += 1;
        }

        Ok(EventsPage { events: filtered_events, continuation_token: None })
    }
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
pub fn filter_events_by_params<'a, 'b: 'a>(
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
