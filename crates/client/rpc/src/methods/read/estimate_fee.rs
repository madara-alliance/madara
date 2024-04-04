use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_hashers::HasherT;
use mp_transactions::UserTransaction;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BroadcastedTransaction, FeeEstimate};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Estimate the fee associated with transaction
///
/// # Arguments
///
/// * `request` - starknet transaction request
/// * `block_id` - hash of the requested block, number (height), or tag
///
/// # Returns
///
/// * `fee_estimate` - fee estimate in gwei
pub async fn estimate_fee<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    request: Vec<BroadcastedTransaction>,
    block_id: BlockId,
) -> RpcResult<Vec<FeeEstimate>>
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
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let transactions =
        request.into_iter().map(|tx| tx.try_into()).collect::<Result<Vec<UserTransaction>, _>>().map_err(|e| {
            error!("Failed to convert BroadcastedTransaction to UserTransaction: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    let fee_estimates = starknet
        .client
        .runtime_api()
        .estimate_fee(substrate_block_hash, transactions)
        .map_err(|e| {
            error!("Request parameters error: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            error!("Failed to call function: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    let estimates = fee_estimates
            .into_iter()
			// FIXME: https://github.com/keep-starknet-strange/madara/issues/329
            .map(|x| FeeEstimate { gas_price: 10, gas_consumed: x.1, overall_fee: x.0 })
            .collect();

    Ok(estimates)
}
