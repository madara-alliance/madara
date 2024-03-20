use jsonrpsee::core::{async_trait, RpcResult};
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer, EstimateMessageFeeServer};
use mc_rpc_core::ChainIdServer;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{BlockId, FeeEstimate, MsgFromL1};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> EstimateMessageFeeServer for Starknet<A, B, BE, G, C, P, H>
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
    /// Estimate the L2 fee of a message sent on L1
    ///
    /// # Arguments
    ///
    /// * `message` - the message to estimate
    /// * `block_id` - hash, number (height), or tag of the requested block
    ///
    /// # Returns
    ///
    /// * `FeeEstimate` - the fee estimation (gas consumed, gas price, overall fee, unit)
    ///
    /// # Errors
    ///
    /// BlockNotFound : If the specified block does not exist.
    /// ContractNotFound : If the specified contract address does not exist.
    /// ContractError : If there is an error with the contract.
    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;
        let chain_id = Felt252Wrapper(self.chain_id()?.0);

        let message = message.try_into().map_err(|e| {
            error!("Failed to convert MsgFromL1 to UserTransaction: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

        let fee_estimate = self
            .client
            .runtime_api()
            .estimate_message_fee(substrate_block_hash, message)
            .map_err(|e| {
                error!("Runtime api error: {e}");
                StarknetRpcApiError::InternalServerError
            })?
            .map_err(|e| {
                error!("function execution failed: {:#?}", e);
                StarknetRpcApiError::ContractError
            })?;

        let estimate = FeeEstimate {
            gas_price: fee_estimate.0.try_into().map_err(|_| StarknetRpcApiError::InternalServerError)?,
            gas_consumed: fee_estimate.2,
            overall_fee: fee_estimate.1,
        };

        Ok(estimate)
    }
}
