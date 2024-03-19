use jsonrpsee::core::{async_trait, RpcResult};
use mc_genesis_data_provider::GenesisProvider;
use log::error;
pub use mc_rpc_core::utils::*;
use mc_rpc_core::BlockHashAndNumberServer;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool_api::TransactionPool;
use sc_transaction_pool::ChainApi;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
pub use mc_rpc_core::BlockNumberServer;
use starknet_core::types::{BlockHashAndNumber, FieldElement};
use crate::errors::StarknetRpcApiError;
use crate::Starknet;

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> BlockHashAndNumberServer for Starknet<A, B, BE, G, C, P, H>
where
    A: ChainApi<Block = B> + 'static,
    B: BlockT,
    P: TransactionPool<Block = B> + 'static,
    BE: Backend<B> + 'static,
    C: HeaderBackend<B> + BlockBackend<B> + StorageProvider<B, BE> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B> + ConvertTransactionRuntimeApi<B>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    /// Get the Most Recent Accepted Block Hash and Number
    ///
    /// ### Arguments
    ///
    /// This function does not take any arguments.
    ///
    /// ### Returns
    ///
    /// * `block_hash_and_number` - A tuple containing the latest block hash and number of the
    ///   current network.
    fn block_hash_and_number(&self) -> RpcResult<BlockHashAndNumber> {
        let block_number = self.current_block_number()?;
        let block_hash = self.current_block_hash().map_err(|e| {
            error!("Failed to retrieve the current block hash: {}", e);
            StarknetRpcApiError::NoBlocks
        })?;

        Ok(BlockHashAndNumber {
            block_hash: FieldElement::from_byte_slice_be(block_hash.as_bytes()).unwrap(),
            block_number,
        })
    }
}