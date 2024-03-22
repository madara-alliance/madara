//! Starknet RPC server API implementation
//!
//! It uses the madara client and backend in order to answer queries.

mod constants;
mod errors;
mod events;
mod madara_backend_client;
mod methods;
mod types;
mod utils;

use std::marker::PhantomData;
use std::sync::Arc;

use errors::StarknetRpcApiError;
use jsonrpsee::core::RpcResult;
use log::error;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use mc_storage::OverrideHandle;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_network_sync::SyncingService;
use sc_transaction_pool::{ChainApi, Pool};
use sp_api::ProvideRuntimeApi;
use sp_arithmetic::traits::UniqueSaturatedInto;
use sp_blockchain::HeaderBackend;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkHash;
use starknet_core::types::{BlockId, StateDiff};

use crate::methods::get_block::{
    get_block_with_tx_hashes_finalized, get_block_with_tx_hashes_pending, get_block_with_txs_finalized,
    get_block_with_txs_pending,
};

/// A Starknet RPC server for Madara
#[allow(dead_code)]
pub struct Starknet<A: ChainApi, B: BlockT, BE, G, C, P, H> {
    client: Arc<C>,
    backend: Arc<mc_db::Backend<B>>,
    overrides: Arc<OverrideHandle<B>>,
    #[allow(dead_code)]
    pool: Arc<P>,
    #[allow(dead_code)]
    graph: Arc<Pool<A>>,
    sync_service: Arc<SyncingService<B>>,
    starting_block: <<B>::Header as HeaderT>::Number,
    #[allow(dead_code)]
    genesis_provider: Arc<G>,
    _marker: PhantomData<(B, BE, H)>,
}

/// Constructor for A Starknet RPC server for Madara
/// # Arguments
// * `client` - The Madara client
// * `backend` - The Madara backend
// * `overrides` - The OverrideHandle
// * `sync_service` - The Substrate client sync service
// * `starting_block` - The starting block for the syncing
// * `hasher` - The hasher used by the runtime
//
// # Returns
// * `Self` - The actual Starknet struct
#[allow(clippy::too_many_arguments)]
impl<A: ChainApi, B: BlockT, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H> {
    pub fn new(
        client: Arc<C>,
        backend: Arc<mc_db::Backend<B>>,
        overrides: Arc<OverrideHandle<B>>,
        pool: Arc<P>,
        graph: Arc<Pool<A>>,
        sync_service: Arc<SyncingService<B>>,
        starting_block: <<B>::Header as HeaderT>::Number,
        genesis_provider: Arc<G>,
    ) -> Self {
        Self {
            client,
            backend,
            overrides,
            pool,
            graph,
            sync_service,
            starting_block,
            genesis_provider,
            _marker: PhantomData,
        }
    }
}

impl<A: ChainApi, B, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H>
where
    B: BlockT,
    C: HeaderBackend<B> + 'static,
{
    pub fn current_block_number(&self) -> RpcResult<u64> {
        Ok(UniqueSaturatedInto::<u64>::unique_saturated_into(self.client.info().best_number))
    }
}

impl<A: ChainApi, B, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H>
where
    B: BlockT,
    C: HeaderBackend<B> + 'static,
{
    pub fn current_spec_version(&self) -> RpcResult<String> {
        Ok("0.5.1".to_string())
    }
}

impl<A: ChainApi, B, BE, G, C, P, H> Starknet<A, B, BE, G, C, P, H>
where
    B: BlockT,
    C: HeaderBackend<B> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B>,
    H: HasherT + Send + Sync + 'static,
{
    pub fn current_block_hash(&self) -> Result<H256, StarknetRpcApiError> {
        let substrate_block_hash = self.client.info().best_hash;

        let starknet_block = match get_block_by_block_hash(self.client.as_ref(), substrate_block_hash) {
            Ok(block) => block,
            Err(_) => return Err(StarknetRpcApiError::BlockNotFound),
        };
        Ok(starknet_block.header().hash::<H>().into())
    }

    /// Returns the substrate block hash corresponding to the given Starknet block id
    fn substrate_block_hash_from_starknet_block(&self, block_id: BlockId) -> Result<B::Hash, StarknetRpcApiError> {
        match block_id {
            BlockId::Hash(h) => {
                madara_backend_client::load_hash(self.client.as_ref(), &self.backend, Felt252Wrapper::from(h).into())
                    .map_err(|e| {
                        error!("Failed to load Starknet block hash for Substrate block with hash '{h}': {e}");
                        StarknetRpcApiError::BlockNotFound
                    })?
            }
            BlockId::Number(n) => self
                .client
                .hash(UniqueSaturatedInto::unique_saturated_into(n))
                .map_err(|_| StarknetRpcApiError::BlockNotFound)?,
            BlockId::Tag(_) => Some(self.client.info().best_hash),
        }
        .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    /// Helper function to get the substrate block number from a Starknet block id
    ///
    /// # Arguments
    ///
    /// * `block_id` - The Starknet block id
    ///
    /// # Returns
    ///
    /// * `u64` - The substrate block number
    fn substrate_block_number_from_starknet_block(&self, block_id: BlockId) -> Result<u64, StarknetRpcApiError> {
        // Short circuit on block number
        if let BlockId::Number(x) = block_id {
            return Ok(x);
        }

        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id)?;

        let starknet_block = match get_block_by_block_hash(self.client.as_ref(), substrate_block_hash) {
            Ok(block) => block,
            Err(_) => return Err(StarknetRpcApiError::BlockNotFound),
        };

        Ok(starknet_block.header().block_number)
    }

    /// Returns a list of all transaction hashes in the given block.
    ///
    /// # Arguments
    ///
    /// * `block_hash` - The hash of the block containing the transactions (starknet block).
    fn get_cached_transaction_hashes(&self, block_hash: StarkHash) -> Option<Vec<StarkHash>> {
        self.backend.mapping().cached_transaction_hashes_from_block_hash(block_hash).unwrap_or_else(|err| {
            error!("Failed to read from cache: {err}");
            None
        })
    }

    /// Returns the state diff for the given block.
    ///
    /// # Arguments
    ///
    /// * `starknet_block_hash` - The hash of the block containing the state diff (starknet block).
    fn get_state_diff(&self, starknet_block_hash: &BlockHash) -> Result<StateDiff, StarknetRpcApiError> {
        let state_diff = self.backend.da().state_diff(starknet_block_hash).map_err(|e| {
            error!("Failed to retrieve state diff from cache for block with hash {}: {e}", starknet_block_hash);
            StarknetRpcApiError::InternalServerError
        })?;

        let rpc_state_diff = to_rpc_state_diff(state_diff);

        Ok(rpc_state_diff)
    }
}
