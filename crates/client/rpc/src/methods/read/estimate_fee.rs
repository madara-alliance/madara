use blockifier::transaction::account_transaction::AccountTransaction;
use jsonrpsee::core::RpcResult;
use mp_hashers::HasherT;
use mp_transactions::from_broadcasted_transactions::ToAccountTransaction;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BroadcastedTransaction, FeeEstimate, SimulationFlagForEstimateFee};

use crate::errors::StarknetRpcApiError;
use crate::utils::execution::block_context;
use crate::{utils, Starknet};

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
pub async fn estimate_fee<BE, C, H>(
    starknet: &Starknet<BE, C, H>,
    request: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    block_id: BlockId,
) -> RpcResult<Vec<FeeEstimate>>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id)?;

    let block_context = block_context(starknet.client.as_ref(), substrate_block_hash)?;

    let transactions = request
        .into_iter()
        .map(|tx| tx.to_account_transaction())
        .collect::<Result<Vec<AccountTransaction>, _>>()
        .map_err(|e| {
            log::error!("Failed to convert BroadcastedTransaction to AccountTransaction: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    let account_transactions: Vec<AccountTransaction> =
        transactions.into_iter().map(AccountTransaction::from).collect();

    let validate = !simulation_flags.contains(&SimulationFlagForEstimateFee::SkipValidate);

    let fee_estimates =
        utils::execution::estimate_fee(account_transactions, validate, &block_context).map_err(|e| {
            log::error!("Failed to call function: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    Ok(fee_estimates)
}
