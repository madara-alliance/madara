use dp_transactions::broadcasted_to_blockifier;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{BlockId, BroadcastedTransaction, FeeEstimate, SimulationFlagForEstimateFee};

use crate::Starknet;
use crate::{errors::StarknetRpcApiError, methods::trace::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW};
use dc_exec::{block_context, execute_transactions, execution_result_to_fee_estimate};

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
    let chain_id = starknet.chain_id();

    if block_info.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion.into());
    }

    let block_context = block_context(block_info.header(), &chain_id).map_err(|e| {
        log::error!("Failed to create block context: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    let transactions =
        request.into_iter().map(|tx| broadcasted_to_blockifier(tx, chain_id)).collect::<Result<Vec<_>, _>>().map_err(
            |e| {
                log::error!("Failed to convert BroadcastedTransaction to AccountTransaction: {e}");
                StarknetRpcApiError::InternalServerError
            },
        )?;

    let validate = !simulation_flags.contains(&SimulationFlagForEstimateFee::SkipValidate);

    let execution_results = execute_transactions(
        starknet.clone_backend(),
        vec![],
        transactions.into_iter(),
        &block_context,
        validate,
        true,
    )
    .map_err(|e| {
        log::error!("Failed to execute fee transaction: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    let fee_estimates =
        execution_results.iter().map(|result| execution_result_to_fee_estimate(result, &block_context)).collect();

    Ok(fee_estimates)
}
