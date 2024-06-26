use blockifier::transaction::account_transaction::AccountTransaction;
use dp_transactions::broadcasted_to_blockifier;
use jsonrpsee::core::RpcResult;
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
pub async fn estimate_fee(
    starknet: &Starknet,
    request: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    block_id: BlockId,
) -> RpcResult<Vec<FeeEstimate>> {
    let block_info = starknet.get_block_info(block_id)?;
    let block_context = block_context(starknet, &block_info)?;
    let chain_id = starknet.chain_id()?;

    let transactions =
        request.into_iter().map(|tx| broadcasted_to_blockifier(tx, chain_id)).collect::<Result<Vec<_>, _>>().map_err(
            |e| {
                log::error!("Failed to convert BroadcastedTransaction to AccountTransaction: {e}");
                StarknetRpcApiError::InternalServerError
            },
        )?;

    let account_transactions: Vec<AccountTransaction> =
        transactions.into_iter().map(AccountTransaction::from).collect();

    let validate = !simulation_flags.contains(&SimulationFlagForEstimateFee::SkipValidate);

    let fee_estimates = utils::execution::estimate_fee(starknet, account_transactions, validate, &block_context)
        .map_err(|e| {
            log::error!("Failed to call function: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    Ok(fee_estimates)
}
