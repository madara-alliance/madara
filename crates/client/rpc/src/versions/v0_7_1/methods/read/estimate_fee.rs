use std::sync::Arc;

use starknet_core::types::{BlockId, BroadcastedTransaction, FeeEstimate, SimulationFlagForEstimateFee};

use mc_exec::ExecutionContext;
use mp_transactions::broadcasted_to_blockifier;

use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::ResultExt;
use crate::versions::v0_7_1::methods::trace::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
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
pub async fn estimate_fee(
    starknet: &Starknet,
    request: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<FeeEstimate>> {
    let block_info = starknet.get_block_info(&block_id)?;
    let starknet_version = *block_info.protocol_version();

    if starknet_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let exec_context = ExecutionContext::new_in_block(Arc::clone(&starknet.backend), &block_info)?;

    let transactions = request
        .into_iter()
        .map(|tx| broadcasted_to_blockifier(tx, starknet.chain_id(), starknet_version).map(|(tx, _)| tx))
        .collect::<Result<Vec<_>, _>>()
        .or_internal_server_error("Failed to convert BroadcastedTransaction to AccountTransaction")?;

    let validate = !simulation_flags.contains(&SimulationFlagForEstimateFee::SkipValidate);

    let execution_results = exec_context.re_execute_transactions([], transactions, true, validate)?;

    let fee_estimates =
        execution_results.iter().map(|result| exec_context.execution_result_to_fee_estimate(result)).collect();

    Ok(fee_estimates)
}
